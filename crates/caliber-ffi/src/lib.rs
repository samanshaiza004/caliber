//! The experimental Caliber C ABI.
//!
//! This crate intentionally exposes a very small, versioned function table.
//! The application contract is opaque bytes: Caliber does not define a domain
//! schema and does not expose Rust containers, references, or callbacks.
//!
//! # Safety and ownership
//!
//! * `CaliberContext` is created and destroyed by this crate.  A context is
//!   `Send + Sync` for concurrent operation calls, but destruction requires
//!   exclusive ownership and must not race with another operation.
//! * Command bytes are copied synchronously during `dispatch`; the caller may
//!   release its input immediately after the function returns.
//! * State and resource reads return an immutable lease.  The pointed-to bytes
//!   remain valid until the matching release function is called.  Leases keep
//!   their bytes alive after context destruction.
//! * Telemetry reads copy into caller-owned storage.  No pointer returned by a
//!   telemetry function outlives the call.
//! * Null pointers are accepted only where explicitly documented.  A non-zero
//!   length always requires a non-null pointer and must fit in `isize`.
//! * Errors are returned as `CaliberStatus`; no Rust panic is allowed to cross
//!   an ABI boundary.  Invalid non-null pointers are still a caller contract
//!   violation, as they are in every C API.
//!
//! The transport uses mutexes and may allocate while dispatching or publishing.
//! None of this ABI is suitable for an audio callback or any other hard
//! realtime thread.  A realtime producer must use a separately audited,
//! non-blocking mechanism and publish a bounded telemetry snapshot to a
//! non-realtime consumer.
//!
//! This private ABI intentionally uses the target platform's `usize` for byte
//! sizes and telemetry words.  Foreign callers must match the target's pointer
//! width and use the reported `value_size`; this is not yet a cross-architecture
//! stable wire ABI.

use caliber_core::{
    ControlQueue, Error as CoreError, LatestTelemetry, ResourceHandle, ResourceRegistry,
    StatePublisher,
};
use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The only ABI table version implemented by this experiment.
pub const CALIBER_ABI_VERSION_1: u32 = 1;
/// A conservative default for command packets.
pub const CALIBER_DEFAULT_MAX_COMMAND_BYTES: usize =
    caliber_core::DEFAULT_MAX_CONTROL_MESSAGE_BYTES;
/// A conservative default for state publications.
pub const CALIBER_DEFAULT_MAX_PUBLICATION_BYTES: usize = caliber_core::DEFAULT_MAX_STATE_BYTES;
/// A conservative default for immutable resources.
pub const CALIBER_DEFAULT_MAX_RESOURCE_BYTES: usize = caliber_core::DEFAULT_MAX_RESOURCE_BYTES;
/// A conservative default for one telemetry payload.
pub const CALIBER_DEFAULT_TELEMETRY_WIDTH: usize = caliber_core::DEFAULT_TELEMETRY_WIDTH;
/// A bounded command queue prevents a foreign caller from growing memory
/// without limit while the application side is stalled.
pub const CALIBER_DEFAULT_MAX_PENDING_COMMANDS: usize = 256;

/// Hard upper bound for a configured command packet.  This is deliberately
/// independent of the default: a foreign caller may select a smaller limit,
/// but cannot use configuration to request an unbounded allocation.
pub const CALIBER_HARD_MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
/// Hard upper bound for one state publication.
pub const CALIBER_HARD_MAX_PUBLICATION_BYTES: usize = 16 * 1024 * 1024;
/// Hard upper bound for one immutable resource.
pub const CALIBER_HARD_MAX_RESOURCE_BYTES: usize = 256 * 1024 * 1024;
/// Hard upper bound for resource slots and pending commands.
pub const CALIBER_HARD_MAX_RESOURCES: usize = 65_536;
pub const CALIBER_HARD_MAX_PENDING_COMMANDS: usize = 16_384;
/// Hard upper bound for a fixed-width telemetry sample.
pub const CALIBER_HARD_MAX_TELEMETRY_WIDTH: usize = 4_096;

/// A result code suitable for C callers.  Values are part of the experimental
/// ABI and must not be reordered.
#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaliberStatus {
    Ok = 0,
    InvalidArgument = 1,
    InvalidHandle = 2,
    BufferTooSmall = 3,
    LimitExceeded = 4,
    NotFound = 5,
    Stale = 6,
    Unavailable = 7,
    QueueFull = 8,
    UnsupportedVersion = 9,
    Internal = 10,
}

/// Caller-provided context limits.  `struct_size` permits a future table to
/// append fields without making old callers initialize them.  All size and
/// count fields use target-platform `usize`; the experimental ABI is therefore
/// platform-width-specific until an explicit-width representation is proven
/// necessary.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CaliberContextConfig {
    pub struct_size: u32,
    pub max_command_bytes: usize,
    pub max_publication_bytes: usize,
    pub max_resource_bytes: usize,
    pub max_resources: usize,
    pub telemetry_width: usize,
    pub max_pending_commands: usize,
}

impl Default for CaliberContextConfig {
    fn default() -> Self {
        Self {
            struct_size: std::mem::size_of::<Self>() as u32,
            max_command_bytes: CALIBER_DEFAULT_MAX_COMMAND_BYTES,
            max_publication_bytes: CALIBER_DEFAULT_MAX_PUBLICATION_BYTES,
            max_resource_bytes: CALIBER_DEFAULT_MAX_RESOURCE_BYTES,
            max_resources: caliber_core::DEFAULT_MAX_RESOURCES,
            telemetry_width: CALIBER_DEFAULT_TELEMETRY_WIDTH,
            max_pending_commands: CALIBER_DEFAULT_MAX_PENDING_COMMANDS,
        }
    }
}

/// An immutable state publication leased from a context.
#[repr(C)]
#[derive(Debug)]
pub struct CaliberStatePublication {
    pub revision: u64,
    pub schema: u32,
    pub reserved: u32,
    pub data: *const u8,
    pub len: usize,
    pub lease: *mut c_void,
}

impl Default for CaliberStatePublication {
    fn default() -> Self {
        Self {
            revision: 0,
            schema: 0,
            reserved: 0,
            data: ptr::null(),
            len: 0,
            lease: ptr::null_mut(),
        }
    }
}

/// An immutable resource view leased from a context.
#[repr(C)]
#[derive(Debug)]
pub struct CaliberResourceView {
    pub resource_id: u64,
    pub generation: u64,
    pub data: *const u8,
    pub len: usize,
    pub lease: *mut c_void,
}

impl Default for CaliberResourceView {
    fn default() -> Self {
        Self {
            resource_id: 0,
            generation: 0,
            data: ptr::null(),
            len: 0,
            lease: ptr::null_mut(),
        }
    }
}

/// Read-only latest-value telemetry metadata.  The payload is copied into a
/// caller buffer by `caliber_context_read_latest_telemetry`.  `value_size` is
/// the target-platform `usize` width for each value in this experimental ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CaliberTelemetryInfo {
    pub sequence: u64,
    pub schema: u32,
    pub reserved: u32,
    pub value_count: usize,
    pub value_size: usize,
}

/// Versioned C function table.  A caller must check `abi_version` and
/// `struct_size` before reading fields.  Function pointers are all optional in
/// the C representation only for forward-compatible table truncation; version
/// 1 returns every pointer below.
#[repr(C)]
pub struct CaliberApiV1 {
    pub abi_version: u32,
    pub struct_size: u32,
    pub context_create: Option<
        unsafe extern "C" fn(
            *const CaliberContextConfig,
            *mut *mut CaliberContext,
        ) -> CaliberStatus,
    >,
    pub context_destroy: Option<unsafe extern "C" fn(*mut CaliberContext)>,
    pub context_dispatch:
        Option<unsafe extern "C" fn(*const CaliberContext, *const u8, usize) -> CaliberStatus>,
    pub context_peek_command:
        Option<unsafe extern "C" fn(*const CaliberContext, *mut usize) -> CaliberStatus>,
    pub context_take_command: Option<
        unsafe extern "C" fn(*const CaliberContext, *mut u8, usize, *mut usize) -> CaliberStatus,
    >,
    pub context_publish_state: Option<
        unsafe extern "C" fn(
            *const CaliberContext,
            u32,
            *const u8,
            usize,
            *mut u64,
        ) -> CaliberStatus,
    >,
    pub context_read_latest_state: Option<
        unsafe extern "C" fn(*const CaliberContext, *mut CaliberStatePublication) -> CaliberStatus,
    >,
    pub state_publication_release: Option<unsafe extern "C" fn(*mut CaliberStatePublication)>,
    pub context_map_resource: Option<
        unsafe extern "C" fn(
            *const CaliberContext,
            u64,
            u64,
            *mut CaliberResourceView,
        ) -> CaliberStatus,
    >,
    pub resource_release: Option<unsafe extern "C" fn(*mut CaliberResourceView)>,
    pub context_publish_resource: Option<
        unsafe extern "C" fn(
            *const CaliberContext,
            *const u8,
            usize,
            *mut u64,
            *mut u64,
        ) -> CaliberStatus,
    >,
    pub context_release_resource:
        Option<unsafe extern "C" fn(*const CaliberContext, u64, u64) -> CaliberStatus>,
    pub context_publish_telemetry:
        Option<unsafe extern "C" fn(*const CaliberContext, *const usize, usize) -> CaliberStatus>,
    pub context_read_latest_telemetry: Option<
        unsafe extern "C" fn(
            *const CaliberContext,
            *mut usize,
            usize,
            *mut CaliberTelemetryInfo,
        ) -> CaliberStatus,
    >,
    pub context_wake_sequence:
        Option<unsafe extern "C" fn(*const CaliberContext, *mut u64) -> CaliberStatus>,
}

/// The opaque context type used by the C ABI.
#[repr(C)]
pub struct CaliberContext {
    inner: ContextInner,
}

struct ContextInner {
    limits: Limits,
    control: ControlQueue,
    command_peek: Mutex<Option<Vec<u8>>>,
    state: StatePublisher,
    resources: ResourceRegistry,
    resource_slots: Mutex<ResourceSlots>,
    telemetry: LatestTelemetry,
    wake_sequence: AtomicU64,
}

#[derive(Clone, Copy)]
struct Limits {
    command_bytes: usize,
    publication_bytes: usize,
    resource_bytes: usize,
    max_resources: usize,
    telemetry_width: usize,
    pending_commands: usize,
}

struct StateLease {
    #[allow(dead_code)]
    publication: Arc<caliber_core::StatePublication>,
}

struct ResourceLease {
    #[allow(dead_code)]
    resource_id: u64,
    #[allow(dead_code)]
    view: caliber_core::ResourceView,
}

#[derive(Clone, Copy)]
struct ResourceSlot {
    handle: ResourceHandle,
    generation: u64,
    live: bool,
}

/// Bounded external resource-id bookkeeping.  Released ids remain as
/// tombstones so a later publication can reuse their slot with a new
/// generation instead of growing the map forever.
struct ResourceSlots {
    entries: HashMap<u64, ResourceSlot>,
    max_entries: usize,
    next_id: u64,
}

impl ResourceSlots {
    fn new(max_entries: usize) -> Result<Self, CaliberStatus> {
        let mut entries = HashMap::new();
        entries
            .try_reserve(max_entries)
            .map_err(|_| CaliberStatus::Internal)?;
        Ok(Self {
            entries,
            max_entries,
            next_id: 1,
        })
    }

    fn reserve_slot(&mut self) -> Result<(u64, u64), CaliberStatus> {
        if let Some((&id, slot)) = self.entries.iter_mut().find(|(_, slot)| !slot.live) {
            let generation = slot
                .generation
                .checked_add(1)
                .ok_or(CaliberStatus::Internal)?;
            return Ok((id, generation));
        }
        if self.entries.len() >= self.max_entries {
            return Err(CaliberStatus::QueueFull);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(CaliberStatus::Internal)?;
        Ok((id, 1))
    }
}

impl CaliberContext {
    fn new(config: CaliberContextConfig) -> Result<Box<Self>, CaliberStatus> {
        let limits = Limits::from_config(config)?;
        Ok(Box::new(Self {
            inner: ContextInner {
                limits,
                control: ControlQueue::new(limits.pending_commands, limits.command_bytes)
                    .map_err(map_core_error)?,
                command_peek: Mutex::new(None),
                state: StatePublisher::new(limits.publication_bytes).map_err(map_core_error)?,
                resources: ResourceRegistry::new(limits.max_resources, limits.resource_bytes)
                    .map_err(map_core_error)?,
                resource_slots: Mutex::new(ResourceSlots::new(limits.max_resources)?),
                telemetry: LatestTelemetry::new(limits.telemetry_width).map_err(map_core_error)?,
                wake_sequence: AtomicU64::new(0),
            },
        }))
    }

    fn dispatch(&self, bytes: &[u8]) -> CaliberStatus {
        if bytes.len() > self.inner.limits.command_bytes {
            return CaliberStatus::LimitExceeded;
        }
        match self.inner.control.try_push(bytes) {
            Ok(()) => {
                self.bump_wake();
                CaliberStatus::Ok
            }
            Err(error) => map_core_error(error),
        }
    }

    fn peek_command_size(&self) -> Result<usize, CaliberStatus> {
        let mut peek = self
            .inner
            .command_peek
            .lock()
            .map_err(|_| CaliberStatus::Internal)?;
        if peek.is_none() {
            *peek = self.inner.control.try_pop();
        }
        peek.as_ref()
            .map(Vec::len)
            .ok_or(CaliberStatus::Unavailable)
    }

    fn take_command(&self, out: &mut [u8]) -> Result<usize, CaliberStatus> {
        let mut peek = self
            .inner
            .command_peek
            .lock()
            .map_err(|_| CaliberStatus::Internal)?;
        if peek.is_none() {
            *peek = self.inner.control.try_pop();
        }
        let command = peek.as_ref().ok_or(CaliberStatus::Unavailable)?;
        if out.len() < command.len() {
            return Err(CaliberStatus::BufferTooSmall);
        }
        let command = peek.take().expect("peeked command remains present");
        out[..command.len()].copy_from_slice(&command);
        Ok(command.len())
    }

    fn read_state(&self) -> Result<CaliberStatePublication, CaliberStatus> {
        let publication = self.inner.state.read().ok_or(CaliberStatus::Unavailable)?;
        let lease = Box::new(StateLease {
            publication: publication.clone(),
        });
        let lease = Box::into_raw(lease);
        Ok(CaliberStatePublication {
            revision: publication.revision(),
            schema: publication.schema(),
            reserved: 0,
            data: publication.payload().as_ptr(),
            len: publication.payload().len(),
            lease: lease.cast(),
        })
    }

    fn map_resource(
        &self,
        resource_id: u64,
        generation: u64,
    ) -> Result<CaliberResourceView, CaliberStatus> {
        let handle = {
            let slots = self
                .inner
                .resource_slots
                .lock()
                .map_err(|_| CaliberStatus::Internal)?;
            let slot = slots
                .entries
                .get(&resource_id)
                .ok_or(CaliberStatus::NotFound)?;
            if !slot.live || slot.generation != generation {
                return Err(CaliberStatus::Stale);
            }
            slot.handle
        };
        let view = match self.inner.resources.get(handle) {
            Ok(view) => view,
            Err(CoreError::StaleResource(_)) => return Err(CaliberStatus::Stale),
            Err(CoreError::UnknownResource(_)) => return Err(CaliberStatus::NotFound),
            Err(error) => return Err(map_core_error(error)),
        };
        let lease = Box::new(ResourceLease {
            resource_id,
            view: view.clone(),
        });
        let lease = Box::into_raw(lease);
        Ok(CaliberResourceView {
            resource_id,
            generation,
            data: view.bytes().as_ptr(),
            len: view.bytes().len(),
            lease: lease.cast(),
        })
    }

    fn read_telemetry(&self, out: &mut [usize]) -> Result<CaliberTelemetryInfo, CaliberStatus> {
        if out.len() != self.inner.limits.telemetry_width {
            return Err(CaliberStatus::BufferTooSmall);
        }
        let sequence = match self.inner.telemetry.read_into(out) {
            Ok(sequence) => sequence,
            Err(CoreError::NoTelemetry) => return Err(CaliberStatus::Unavailable),
            Err(error) => return Err(map_core_error(error)),
        };
        Ok(CaliberTelemetryInfo {
            sequence,
            schema: 0,
            reserved: 0,
            value_count: out.len(),
            value_size: std::mem::size_of::<usize>(),
        })
    }

    fn telemetry_info(&self) -> Result<CaliberTelemetryInfo, CaliberStatus> {
        let sequence = self.inner.telemetry.sequence();
        if sequence == 0 {
            return Err(CaliberStatus::Unavailable);
        }
        Ok(CaliberTelemetryInfo {
            sequence,
            schema: 0,
            reserved: 0,
            value_count: self.inner.limits.telemetry_width,
            value_size: std::mem::size_of::<usize>(),
        })
    }

    fn bump_wake(&self) {
        // Wrapping is intentional: callers compare equality/change, not
        // arithmetic distance, and a u64 wrap is practically unreachable.
        self.inner.wake_sequence.fetch_add(1, Ordering::Release);
    }

    /// Publish a state payload from a native core adapter.  This is a Rust
    /// integration seam, not a second C wire format; the exported ABI only
    /// exposes the resulting immutable publication.
    pub fn publish_state(&self, schema: u32, bytes: &[u8]) -> Result<u64, CaliberStatus> {
        if schema == 0 {
            return Err(CaliberStatus::InvalidArgument);
        }
        match self.inner.state.publish(schema, bytes) {
            Ok(revision) => {
                self.bump_wake();
                Ok(revision)
            }
            Err(error) => Err(map_core_error(error)),
        }
    }

    /// Register immutable bytes and return the opaque ABI id/generation pair
    /// that a frontend may later map.
    pub fn publish_resource(&self, bytes: &[u8]) -> Result<(u64, u64), CaliberStatus> {
        let mut slots = self
            .inner
            .resource_slots
            .lock()
            .map_err(|_| CaliberStatus::Internal)?;
        let (resource_id, generation) = slots.reserve_slot()?;
        let handle = self.inner.resources.insert(bytes).map_err(map_core_error)?;
        slots.entries.insert(
            resource_id,
            ResourceSlot {
                handle,
                generation,
                live: true,
            },
        );
        self.bump_wake();
        Ok((resource_id, generation))
    }

    /// Release the registry's owner reference. Existing mapped leases remain
    /// valid until their matching release operation.
    pub fn release_resource(&self, resource_id: u64, generation: u64) -> CaliberStatus {
        let mut slots = match self.inner.resource_slots.lock() {
            Ok(slots) => slots,
            Err(_) => return CaliberStatus::Internal,
        };
        let Some(slot) = slots.entries.get(&resource_id).copied() else {
            return CaliberStatus::NotFound;
        };
        if !slot.live || slot.generation != generation {
            return CaliberStatus::Stale;
        }
        match self.inner.resources.release(slot.handle) {
            Ok(()) => {
                // Keep the tombstone, but make the bounded slot available for
                // reuse with a new generation.
                if let Some(slot) = slots.entries.get_mut(&resource_id) {
                    slot.live = false;
                }
                self.bump_wake();
                CaliberStatus::Ok
            }
            Err(error) => map_core_error(error),
        }
    }

    /// Publish a fixed-width latest telemetry value through the core slot.
    pub fn publish_telemetry(&self, values: &[usize]) -> CaliberStatus {
        match self.inner.telemetry.publish(values) {
            Ok(_) => {
                self.bump_wake();
                CaliberStatus::Ok
            }
            Err(error) => map_core_error(error),
        }
    }
}

impl Limits {
    fn from_config(config: CaliberContextConfig) -> Result<Self, CaliberStatus> {
        // A zero field means “use the default”, which keeps a zeroed C config
        // useful while still rejecting overflows and impossible queue sizes.
        let limits = Self {
            command_bytes: default_or(config.max_command_bytes, CALIBER_DEFAULT_MAX_COMMAND_BYTES),
            publication_bytes: default_or(
                config.max_publication_bytes,
                CALIBER_DEFAULT_MAX_PUBLICATION_BYTES,
            ),
            resource_bytes: default_or(
                config.max_resource_bytes,
                CALIBER_DEFAULT_MAX_RESOURCE_BYTES,
            ),
            max_resources: default_or(config.max_resources, caliber_core::DEFAULT_MAX_RESOURCES),
            telemetry_width: default_or(config.telemetry_width, CALIBER_DEFAULT_TELEMETRY_WIDTH),
            pending_commands: default_or(
                config.max_pending_commands,
                CALIBER_DEFAULT_MAX_PENDING_COMMANDS,
            ),
        };
        if limits.command_bytes == 0
            || limits.publication_bytes == 0
            || limits.resource_bytes == 0
            || limits.max_resources == 0
            || limits.telemetry_width == 0
            || limits.pending_commands == 0
        {
            return Err(CaliberStatus::InvalidArgument);
        }
        if limits.command_bytes > CALIBER_HARD_MAX_COMMAND_BYTES
            || limits.publication_bytes > CALIBER_HARD_MAX_PUBLICATION_BYTES
            || limits.resource_bytes > CALIBER_HARD_MAX_RESOURCE_BYTES
            || limits.max_resources > CALIBER_HARD_MAX_RESOURCES
            || limits.telemetry_width > CALIBER_HARD_MAX_TELEMETRY_WIDTH
            || limits.pending_commands > CALIBER_HARD_MAX_PENDING_COMMANDS
        {
            return Err(CaliberStatus::LimitExceeded);
        }
        Ok(limits)
    }
}

fn map_core_error(error: CoreError) -> CaliberStatus {
    match error {
        CoreError::TooLarge { .. } => CaliberStatus::LimitExceeded,
        CoreError::Full { .. } | CoreError::TelemetryBusy => CaliberStatus::QueueFull,
        CoreError::StaleResource(_) => CaliberStatus::Stale,
        CoreError::UnknownResource(_) => CaliberStatus::NotFound,
        CoreError::NoTelemetry => CaliberStatus::Unavailable,
        CoreError::WrongSize { .. } | CoreError::InvalidLimit { .. } => {
            CaliberStatus::InvalidArgument
        }
        CoreError::RevisionExhausted | CoreError::InvalidSchemaVersion => {
            CaliberStatus::InvalidArgument
        }
        CoreError::AllocationFailed { .. } => CaliberStatus::Internal,
    }
}

fn default_or(value: usize, default: usize) -> usize {
    if value == 0 { default } else { value }
}

fn checked_input<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], CaliberStatus> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() || len > isize::MAX as usize {
        return Err(CaliberStatus::InvalidArgument);
    }
    // SAFETY: null and isize bounds were checked above.  The caller owns the
    // validity of the non-null allocation for the duration of the call.
    Ok(unsafe { slice::from_raw_parts(ptr, len) })
}

fn checked_usize_input<'a>(ptr: *const usize, len: usize) -> Result<&'a [usize], CaliberStatus> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() || len > isize::MAX as usize {
        return Err(CaliberStatus::InvalidArgument);
    }
    // SAFETY: null and isize bounds were checked above.  The caller owns the
    // validity of the non-null allocation for the duration of the call.
    Ok(unsafe { slice::from_raw_parts(ptr, len) })
}

fn checked_output<'a>(ptr: *mut usize, len: usize) -> Result<&'a mut [usize], CaliberStatus> {
    if len == 0 {
        return Ok(&mut []);
    }
    if ptr.is_null() || len > isize::MAX as usize {
        return Err(CaliberStatus::InvalidArgument);
    }
    // SAFETY: null and isize bounds were checked above.  The caller owns the
    // writable allocation for the duration of the call.
    Ok(unsafe { slice::from_raw_parts_mut(ptr, len) })
}

fn checked_output_bytes<'a>(ptr: *mut u8, len: usize) -> Result<&'a mut [u8], CaliberStatus> {
    if len == 0 {
        return Ok(&mut []);
    }
    if ptr.is_null() || len > isize::MAX as usize {
        return Err(CaliberStatus::InvalidArgument);
    }
    // SAFETY: null and isize bounds were checked above.  The caller owns the
    // writable allocation for the duration of the call.
    Ok(unsafe { slice::from_raw_parts_mut(ptr, len) })
}

fn with_context<'a>(ptr: *const CaliberContext) -> Result<&'a CaliberContext, CaliberStatus> {
    if ptr.is_null() {
        return Err(CaliberStatus::InvalidHandle);
    }
    // SAFETY: a non-null context pointer must have been returned by create and
    // remain alive for the duration of this operation.
    Ok(unsafe { &*ptr })
}

fn read_config(ptr: *const CaliberContextConfig) -> Result<CaliberContextConfig, CaliberStatus> {
    if ptr.is_null() {
        return Ok(CaliberContextConfig::default());
    }
    // Read only the advertised prefix. A foreign caller compiled against a
    // shorter table must never be forced to provide newer trailing fields.
    let base = ptr.cast::<u8>();
    // SAFETY: the first four bytes are the required struct-size field for a
    // non-null config pointer.
    let advertised = unsafe { base.cast::<u32>().read_unaligned() } as usize;
    if advertised == 0 {
        return Ok(CaliberContextConfig::default());
    }
    if advertised < std::mem::size_of::<u32>() {
        return Err(CaliberStatus::InvalidArgument);
    }
    let mut config = CaliberContextConfig {
        struct_size: advertised as u32,
        ..CaliberContextConfig::default()
    };
    macro_rules! field {
        ($name:ident) => {{
            let offset = std::mem::offset_of!(CaliberContextConfig, $name);
            let end = offset
                .checked_add(std::mem::size_of_val(&config.$name))
                .ok_or(CaliberStatus::InvalidArgument)?;
            if advertised >= end {
                // SAFETY: the advertised size covers this complete field.
                config.$name = unsafe { base.add(offset).cast::<usize>().read_unaligned() };
            }
        }};
    }
    field!(max_command_bytes);
    field!(max_publication_bytes);
    field!(max_resource_bytes);
    field!(max_resources);
    field!(telemetry_width);
    field!(max_pending_commands);
    Ok(config)
}

/// Return the versioned table, or null for an unsupported ABI version.
#[unsafe(no_mangle)]
pub extern "C" fn caliber_get_api(version: u32) -> *const CaliberApiV1 {
    if version == CALIBER_ABI_VERSION_1 {
        &API_V1
    } else {
        ptr::null()
    }
}

/// Create a context.  A null config selects defaults.  `out_context` is
/// required and receives ownership of the opaque handle on success.
///
/// # Safety
/// `out_context` must point to writable caller-owned storage. If `config` is
/// non-null it must point to a readable prefix described by `struct_size`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_create(
    config: *const CaliberContextConfig,
    out_context: *mut *mut CaliberContext,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_context.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        // Make failure unambiguous for callers that reused an output slot.
        // SAFETY: out_context was checked non-null and is caller-owned.
        unsafe { *out_context = ptr::null_mut() };
        let config = match read_config(config) {
            Ok(config) => config,
            Err(status) => return status,
        };
        let context = match CaliberContext::new(config) {
            Ok(context) => context,
            Err(status) => return status,
        };
        // SAFETY: out_context was checked non-null and is caller-owned output.
        unsafe { *out_context = Box::into_raw(context) };
        CaliberStatus::Ok
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Destroy a context.  Outstanding state/resource leases remain valid because
/// they own their immutable bytes independently.  The pointer must not be
/// used concurrently and must have come from `caliber_context_create`.
///
/// # Safety
/// `context` must be null or a handle returned by `caliber_context_create`,
/// with no concurrent operation still using it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_destroy(context: *mut CaliberContext) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if context.is_null() {
            return;
        }
        // SAFETY: caller owns this context and has stopped concurrent calls.
        unsafe { drop(Box::from_raw(context)) };
    }));
}

/// Copy an opaque command packet into the bounded control queue.
///
/// # Safety
/// `context` must be a live handle. If `len` is non-zero, `bytes` must point
/// to a readable allocation of at least `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_dispatch(
    context: *const CaliberContext,
    bytes: *const u8,
    len: usize,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let bytes = match checked_input(bytes, len) {
            Ok(bytes) => bytes,
            Err(status) => return status,
        };
        context.dispatch(bytes)
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Return the byte length of the oldest queued command without removing it.
/// `out_len` is caller-owned and required.  The command remains queued until
/// `caliber_context_take_command` succeeds.
///
/// # Safety
/// `context` must be a live handle and `out_len` must point to writable
/// caller-owned storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_peek_command(
    context: *const CaliberContext,
    out_len: *mut usize,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_len.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let len = match context.peek_command_size() {
            Ok(len) => len,
            Err(status) => return status,
        };
        // SAFETY: out_len was checked non-null and is caller-owned output.
        unsafe { *out_len = len };
        CaliberStatus::Ok
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Copy and remove the oldest queued command.  A short destination leaves the
/// command queued and returns `BufferTooSmall`; `out_len` reports the required
/// capacity on that path.
///
/// # Safety
/// `context` must be a live handle, `out_len` must point to writable storage,
/// and a non-zero `capacity` requires `out` to point to a writable allocation
/// of that capacity.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_take_command(
    context: *const CaliberContext,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_len.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let out = match checked_output_bytes(out, capacity) {
            Ok(out) => out,
            Err(status) => return status,
        };
        let required = match context.peek_command_size() {
            Ok(required) => required,
            Err(status) => return status,
        };
        // SAFETY: out_len was checked non-null and is caller-owned output.
        unsafe { *out_len = required };
        if capacity < required {
            return CaliberStatus::BufferTooSmall;
        }
        match context.take_command(out) {
            Ok(actual) => {
                // Keep the reported size authoritative even if a future core
                // implementation changes its queue internals.
                unsafe { *out_len = actual };
                CaliberStatus::Ok
            }
            Err(status) => status,
        }
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Publish an opaque, application-defined state payload. The core assigns a
/// monotonic revision and returns it through `out_revision`.
///
/// # Safety
/// `context` must be a live handle, `out_revision` must point to writable
/// storage, and a non-zero `len` requires `bytes` to point to readable storage
/// of that length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_publish_state(
    context: *const CaliberContext,
    schema: u32,
    bytes: *const u8,
    len: usize,
    out_revision: *mut u64,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_revision.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let bytes = match checked_input(bytes, len) {
            Ok(bytes) => bytes,
            Err(status) => return status,
        };
        let revision = match context.publish_state(schema, bytes) {
            Ok(revision) => revision,
            Err(status) => return status,
        };
        // SAFETY: out_revision was checked non-null and is caller-owned output.
        unsafe { *out_revision = revision };
        CaliberStatus::Ok
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Read the latest coherent state publication.  The result owns a lease and
/// must be released with `caliber_state_publication_release`.
///
/// # Safety
/// `context` must be a live handle and `out` must point to writable
/// caller-owned storage for one view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_read_latest_state(
    context: *const CaliberContext,
    out: *mut CaliberStatePublication,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let publication = match context.read_state() {
            Ok(publication) => publication,
            Err(status) => return status,
        };
        // SAFETY: out was checked non-null and is caller-owned output.
        unsafe { *out = publication };
        CaliberStatus::Ok
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Release a state lease and clear its view.  Null/empty views are harmless.
///
/// # Safety
/// `out` must be null or a view previously filled by
/// `caliber_context_read_latest_state` and not already released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_state_publication_release(out: *mut CaliberStatePublication) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if out.is_null() {
            return;
        }
        // SAFETY: out must be the view returned by the matching read call.
        let out = unsafe { &mut *out };
        if !out.lease.is_null() {
            // SAFETY: lease was produced by Box::into_raw in read_state.
            unsafe { drop(Box::from_raw(out.lease.cast::<StateLease>())) };
        }
        *out = CaliberStatePublication::default();
    }));
}

/// Map an immutable resource by id and exact generation.  The returned view
/// must be released with `caliber_resource_release`.
///
/// # Safety
/// `context` must be a live handle and `out` must point to writable
/// caller-owned storage for one view.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_map_resource(
    context: *const CaliberContext,
    resource_id: u64,
    generation: u64,
    out: *mut CaliberResourceView,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let resource = match context.map_resource(resource_id, generation) {
            Ok(resource) => resource,
            Err(status) => return status,
        };
        // SAFETY: out was checked non-null and is caller-owned output.
        unsafe { *out = resource };
        CaliberStatus::Ok
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Register immutable bytes and return an opaque id/generation pair. The
/// context retains the registry's owner reference until the matching
/// `caliber_context_release_resource` call; mapped views independently retain
/// their bytes until `caliber_resource_release`.
///
/// # Safety
/// `context` must be a live handle, both output pointers must be writable, and
/// a non-zero `len` requires `bytes` to point to readable storage of that
/// length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_publish_resource(
    context: *const CaliberContext,
    bytes: *const u8,
    len: usize,
    out_resource_id: *mut u64,
    out_generation: *mut u64,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_resource_id.is_null() || out_generation.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let bytes = match checked_input(bytes, len) {
            Ok(bytes) => bytes,
            Err(status) => return status,
        };
        let (resource_id, generation) = match context.publish_resource(bytes) {
            Ok(handle) => handle,
            Err(status) => return status,
        };
        // SAFETY: both outputs were checked non-null and are caller-owned.
        unsafe {
            *out_resource_id = resource_id;
            *out_generation = generation;
        }
        CaliberStatus::Ok
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Release the registry's owner reference for a resource generation. Existing
/// mapped leases remain readable after this call.
///
/// # Safety
/// `context` must be a live handle and no concurrent resource-registry
/// shutdown may be in progress.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_release_resource(
    context: *const CaliberContext,
    resource_id: u64,
    generation: u64,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        context.release_resource(resource_id, generation)
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Release a resource lease and clear its view.  Null/empty views are harmless.
///
/// # Safety
/// `out` must be null or a view previously filled by
/// `caliber_context_map_resource` and not already released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_resource_release(out: *mut CaliberResourceView) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if out.is_null() {
            return;
        }
        // SAFETY: out must be the view returned by the matching map call.
        let out = unsafe { &mut *out };
        if !out.lease.is_null() {
            // SAFETY: lease was produced by Box::into_raw in map_resource.
            unsafe { drop(Box::from_raw(out.lease.cast::<ResourceLease>())) };
        }
        *out = CaliberResourceView::default();
    }));
}

/// Publish one fixed-width latest telemetry value. The width is selected at
/// context creation; intermediate values may be overwritten and no history is
/// retained.
///
/// # Safety
/// `context` must be a live handle. If `count` is non-zero, `values` must
/// point to `count` readable `usize` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_publish_telemetry(
    context: *const CaliberContext,
    values: *const usize,
    count: usize,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let values = match checked_usize_input(values, count) {
            Ok(values) => values,
            Err(status) => return status,
        };
        context.publish_telemetry(values)
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Copy the latest telemetry payload into `out`.  `info` is always populated
/// when a telemetry value exists, including on `BufferTooSmall`, so callers can
/// resize once and retry.  A null output is valid only with a zero capacity.
///
/// # Safety
/// `context` must be a live handle, `info` must point to writable storage, and
/// a non-zero capacity requires `out` to point to writable storage for that
/// many `usize` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_read_latest_telemetry(
    context: *const CaliberContext,
    out: *mut usize,
    capacity: usize,
    info: *mut CaliberTelemetryInfo,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if info.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        let out = match checked_output(out, capacity) {
            Ok(out) => out,
            Err(status) => return status,
        };
        let telemetry_info = match context.telemetry_info() {
            Ok(info) => info,
            Err(status) => return status,
        };
        if out.len() != telemetry_info.value_count {
            // Keep the latest-value API fixed-width, as required by the core
            // telemetry slot.  The caller can inspect the configured width in
            // its own context setup before allocating this buffer.
            // SAFETY: info was checked non-null and is caller-owned output.
            unsafe { *info = telemetry_info };
            return CaliberStatus::BufferTooSmall;
        }
        let telemetry_info = match context.read_telemetry(out) {
            Ok(info) => info,
            Err(status) => return status,
        };
        // SAFETY: info was checked non-null and is caller-owned output.
        unsafe { *info = telemetry_info };
        CaliberStatus::Ok
    }))
    .unwrap_or(CaliberStatus::Internal)
}

/// Read the monotonically changing wake sequence.  A change means that at
/// least one command/publication/resource/telemetry event occurred since the
/// caller's last observation; it is not a count of events and may wrap.
///
/// # Safety
/// `context` must be a live handle and `out_sequence` must point to writable
/// caller-owned storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn caliber_context_wake_sequence(
    context: *const CaliberContext,
    out_sequence: *mut u64,
) -> CaliberStatus {
    catch_unwind(AssertUnwindSafe(|| {
        if out_sequence.is_null() {
            return CaliberStatus::InvalidArgument;
        }
        let context = match with_context(context) {
            Ok(context) => context,
            Err(status) => return status,
        };
        // SAFETY: out_sequence was checked non-null and is caller-owned output.
        unsafe { *out_sequence = context.inner.wake_sequence.load(Ordering::Acquire) };
        CaliberStatus::Ok
    }))
    .unwrap_or(CaliberStatus::Internal)
}

static API_V1: CaliberApiV1 = CaliberApiV1 {
    abi_version: CALIBER_ABI_VERSION_1,
    struct_size: std::mem::size_of::<CaliberApiV1>() as u32,
    context_create: Some(caliber_context_create),
    context_destroy: Some(caliber_context_destroy),
    context_dispatch: Some(caliber_context_dispatch),
    context_peek_command: Some(caliber_context_peek_command),
    context_take_command: Some(caliber_context_take_command),
    context_publish_state: Some(caliber_context_publish_state),
    context_read_latest_state: Some(caliber_context_read_latest_state),
    state_publication_release: Some(caliber_state_publication_release),
    context_map_resource: Some(caliber_context_map_resource),
    resource_release: Some(caliber_resource_release),
    context_publish_resource: Some(caliber_context_publish_resource),
    context_release_resource: Some(caliber_context_release_resource),
    context_publish_telemetry: Some(caliber_context_publish_telemetry),
    context_read_latest_telemetry: Some(caliber_context_read_latest_telemetry),
    context_wake_sequence: Some(caliber_context_wake_sequence),
};

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Box<CaliberContext> {
        CaliberContext::new(CaliberContextConfig::default()).expect("default config")
    }

    #[test]
    fn api_table_is_versioned_and_rejects_unknown_versions() {
        let api = caliber_get_api(CALIBER_ABI_VERSION_1);
        assert!(!api.is_null());
        // SAFETY: the pointer is the static table returned above.
        let api = unsafe { &*api };
        assert_eq!(api.abi_version, CALIBER_ABI_VERSION_1);
        assert!(api.struct_size as usize >= std::mem::size_of::<CaliberApiV1>());
        assert!(caliber_get_api(CALIBER_ABI_VERSION_1 + 1).is_null());
    }

    #[test]
    fn command_bytes_are_bounded_and_wake_sequence_changes() {
        let context = context();
        let mut wake = 0;
        assert_eq!(
            unsafe { caliber_context_wake_sequence(&*context, &mut wake) },
            CaliberStatus::Ok
        );
        assert_eq!(context.dispatch(b"hello"), CaliberStatus::Ok);
        let mut after = 0;
        assert_eq!(
            unsafe { caliber_context_wake_sequence(&*context, &mut after) },
            CaliberStatus::Ok
        );
        assert_ne!(wake, after);

        let limits = CaliberContextConfig {
            max_command_bytes: 2,
            ..CaliberContextConfig::default()
        };
        let small = CaliberContext::new(limits).expect("small config");
        assert_eq!(small.dispatch(b"123"), CaliberStatus::LimitExceeded);
    }

    #[test]
    fn state_lease_survives_context_destroy() {
        let context = context();
        assert_eq!(context.publish_state(3, b"state"), Ok(1));
        let mut publication = context.read_state().expect("state");
        assert_eq!(publication.revision, 1);
        assert_eq!(publication.schema, 3);
        // SAFETY: the view came from read_state and points to its lease.
        let bytes = unsafe { slice::from_raw_parts(publication.data, publication.len) };
        assert_eq!(bytes, b"state");
        let raw = Box::into_raw(context);
        // SAFETY: raw came from Box::into_raw and is exclusively owned here.
        unsafe { caliber_context_destroy(raw) };
        assert!(!publication.lease.is_null());
        // SAFETY: release consumes the lease even after context destruction.
        unsafe { caliber_state_publication_release(&mut publication) };
        assert!(publication.lease.is_null());
    }

    #[test]
    fn resource_generation_is_exact_and_release_clears_view() {
        let context = context();
        let (resource_id, generation) = context
            .publish_resource(b"waveform")
            .expect("resource publication");
        assert!(matches!(
            context.map_resource(resource_id, generation - 1),
            Err(CaliberStatus::Stale)
        ));
        let mut view = context
            .map_resource(resource_id, generation)
            .expect("resource");
        // SAFETY: the view came from map_resource and points to its lease.
        let bytes = unsafe { slice::from_raw_parts(view.data, view.len) };
        assert_eq!(bytes, b"waveform");
        // SAFETY: release consumes the lease.
        unsafe { caliber_resource_release(&mut view) };
        assert!(view.data.is_null());
        assert!(view.lease.is_null());
    }

    #[test]
    fn released_resource_slots_are_reused_with_a_new_generation() {
        let config = CaliberContextConfig {
            max_resources: 1,
            ..CaliberContextConfig::default()
        };
        let context = CaliberContext::new(config).expect("bounded config");
        let (resource_id, generation) = context.publish_resource(b"first").expect("first resource");
        assert_eq!(
            context.publish_resource(b"still full"),
            Err(CaliberStatus::QueueFull)
        );
        assert_eq!(
            context.release_resource(resource_id, generation),
            CaliberStatus::Ok
        );
        let (reused_id, reused_generation) = context
            .publish_resource(b"second")
            .expect("reused resource slot");
        assert_eq!(reused_id, resource_id);
        assert!(reused_generation > generation);
        assert!(matches!(
            context.map_resource(resource_id, generation),
            Err(CaliberStatus::Stale)
        ));
        let mut view = context
            .map_resource(reused_id, reused_generation)
            .expect("new generation");
        assert_eq!(view.len, 6);
        // SAFETY: view came from map_resource and is released exactly once.
        unsafe { caliber_resource_release(&mut view) };
    }

    #[test]
    fn telemetry_reports_required_size_without_partial_copy() {
        let context = context();
        assert_eq!(
            context.publish_telemetry(&[44, 8, 7, 6, 5, 4, 3, 2]),
            CaliberStatus::Ok
        );
        let mut info = CaliberTelemetryInfo::default();
        let mut out = [0_usize; 2];
        // SAFETY: pointers refer to live caller-owned storage.
        let status = unsafe {
            caliber_context_read_latest_telemetry(&*context, out.as_mut_ptr(), out.len(), &mut info)
        };
        assert_eq!(status, CaliberStatus::BufferTooSmall);
        assert_eq!(info.sequence, 1);
        assert_eq!(info.value_count, 8);
        assert_eq!(out, [0, 0]);
        let mut out = [0_usize; 8];
        // SAFETY: pointers refer to live caller-owned storage.
        assert_eq!(
            unsafe {
                caliber_context_read_latest_telemetry(
                    &*context,
                    out.as_mut_ptr(),
                    out.len(),
                    &mut info,
                )
            },
            CaliberStatus::Ok
        );
        assert_eq!(&out, &[44, 8, 7, 6, 5, 4, 3, 2]);
    }

    #[test]
    fn application_side_round_trip_uses_same_abi_table() {
        let api = unsafe { &*caliber_get_api(CALIBER_ABI_VERSION_1) };
        let mut raw = ptr::null_mut();
        let create = api.context_create.expect("create");
        // SAFETY: output is writable and the null config selects defaults.
        assert_eq!(unsafe { create(ptr::null(), &mut raw) }, CaliberStatus::Ok);
        assert!(!raw.is_null());

        let command = b"select:9";
        // SAFETY: raw is live and command is readable for this call.
        assert_eq!(
            unsafe {
                api.context_dispatch.expect("dispatch")(raw, command.as_ptr(), command.len())
            },
            CaliberStatus::Ok
        );
        let mut required = 0;
        // SAFETY: raw is live and required is writable.
        assert_eq!(
            unsafe { api.context_peek_command.expect("peek")(raw, &mut required) },
            CaliberStatus::Ok
        );
        assert_eq!(required, command.len());
        let mut short = [0_u8; 1];
        // SAFETY: raw and output are live; the short output is intentionally
        // used to prove the command remains queued.
        assert_eq!(
            unsafe {
                api.context_take_command.expect("take")(
                    raw,
                    short.as_mut_ptr(),
                    short.len(),
                    &mut required,
                )
            },
            CaliberStatus::BufferTooSmall
        );
        let mut taken = [0_u8; 32];
        // SAFETY: raw and output are live and capacity is sufficient.
        assert_eq!(
            unsafe {
                api.context_take_command.expect("take")(
                    raw,
                    taken.as_mut_ptr(),
                    taken.len(),
                    &mut required,
                )
            },
            CaliberStatus::Ok
        );
        assert_eq!(&taken[..required], command);

        let mut revision = 0;
        // SAFETY: raw and state bytes are live; revision is writable.
        assert_eq!(
            unsafe {
                api.context_publish_state.expect("publish state")(
                    raw,
                    7,
                    b"state".as_ptr(),
                    5,
                    &mut revision,
                )
            },
            CaliberStatus::Ok
        );
        assert_eq!(revision, 1);
        let mut publication = CaliberStatePublication::default();
        // SAFETY: raw and publication output are live.
        assert_eq!(
            unsafe { api.context_read_latest_state.expect("read state")(raw, &mut publication) },
            CaliberStatus::Ok
        );
        // SAFETY: the publication lease owns these bytes until release.
        assert_eq!(
            unsafe { slice::from_raw_parts(publication.data, publication.len) },
            b"state"
        );
        // SAFETY: publication came from the matching read function.
        unsafe { api.state_publication_release.expect("release state")(&mut publication) };

        let mut resource_id = 0;
        let mut generation = 0;
        // SAFETY: raw and resource bytes are live; outputs are writable.
        assert_eq!(
            unsafe {
                api.context_publish_resource.expect("publish resource")(
                    raw,
                    b"bulk".as_ptr(),
                    4,
                    &mut resource_id,
                    &mut generation,
                )
            },
            CaliberStatus::Ok
        );
        let mut resource = CaliberResourceView::default();
        // SAFETY: raw and resource output are live.
        assert_eq!(
            unsafe {
                api.context_map_resource.expect("map resource")(
                    raw,
                    resource_id,
                    generation,
                    &mut resource,
                )
            },
            CaliberStatus::Ok
        );
        // SAFETY: resource lease owns these bytes until release.
        assert_eq!(
            unsafe { slice::from_raw_parts(resource.data, resource.len) },
            b"bulk"
        );
        // SAFETY: release the registry owner, then release the mapped lease.
        assert_eq!(
            unsafe {
                api.context_release_resource.expect("release resource")(
                    raw,
                    resource_id,
                    generation,
                )
            },
            CaliberStatus::Ok
        );
        assert_eq!(
            unsafe { api.resource_release.expect("resource lease release")(&mut resource) },
            ()
        );

        let values = [1_usize, 2, 3, 4, 5, 6, 7, 8];
        // SAFETY: values are readable for this call.
        assert_eq!(
            unsafe {
                api.context_publish_telemetry.expect("publish telemetry")(
                    raw,
                    values.as_ptr(),
                    values.len(),
                )
            },
            CaliberStatus::Ok
        );
        let mut observed = [0_usize; 8];
        let mut telemetry = CaliberTelemetryInfo::default();
        // SAFETY: observed and telemetry are writable.
        assert_eq!(
            unsafe {
                api.context_read_latest_telemetry.expect("read telemetry")(
                    raw,
                    observed.as_mut_ptr(),
                    observed.len(),
                    &mut telemetry,
                )
            },
            CaliberStatus::Ok
        );
        assert_eq!(observed, values);

        // SAFETY: raw is the live context returned by create and is now
        // exclusively owned by this test.
        unsafe { api.context_destroy.expect("destroy")(raw) };
    }

    #[test]
    fn short_config_prefix_is_read_without_overreading_new_fields() {
        #[repr(C)]
        struct OldConfig {
            struct_size: u32,
            max_command_bytes: usize,
        }
        let old = OldConfig {
            struct_size: (std::mem::offset_of!(OldConfig, max_command_bytes)
                + std::mem::size_of::<usize>()) as u32,
            max_command_bytes: 3,
        };
        let mut raw = ptr::null_mut();
        // SAFETY: old points to exactly the advertised readable prefix and
        // raw is writable output.
        assert_eq!(
            unsafe { caliber_context_create((&old as *const OldConfig).cast(), &mut raw) },
            CaliberStatus::Ok
        );
        assert_eq!(
            unsafe { caliber_context_dispatch(raw, b"1234".as_ptr(), 4) },
            CaliberStatus::LimitExceeded
        );
        // SAFETY: raw came from create and is exclusively owned here.
        unsafe { caliber_context_destroy(raw) };
    }

    #[test]
    fn oversized_config_is_rejected_before_allocating_context() {
        let config = CaliberContextConfig {
            max_pending_commands: CALIBER_HARD_MAX_PENDING_COMMANDS + 1,
            ..CaliberContextConfig::default()
        };
        let mut raw = ptr::dangling_mut::<CaliberContext>();
        // SAFETY: config and raw are writable/readable caller-owned values.
        assert_eq!(
            unsafe { caliber_context_create(&config, &mut raw) },
            CaliberStatus::LimitExceeded
        );
        assert!(raw.is_null());
    }

    #[test]
    fn null_and_zero_length_rules_are_explicit() {
        let context = context();
        // A null pointer is valid for an empty command.
        assert_eq!(context.dispatch(&[]), CaliberStatus::Ok);
        // A non-empty command requires a pointer.
        let raw = Box::into_raw(context);
        assert_eq!(
            unsafe { caliber_context_dispatch(raw, ptr::null(), 1) },
            CaliberStatus::InvalidArgument
        );
        // SAFETY: raw is still owned by this test after the failed call.
        unsafe { caliber_context_destroy(raw) };
    }
}
