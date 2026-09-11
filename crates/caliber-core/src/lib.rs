//! Small, private experiments for the Caliber application/presentation boundary.
//!
//! This crate deliberately has no GUI, window, renderer, Wasm, async, or FFI
//! dependencies. It contains mechanisms only; it does not define widgets,
//! application state schemas, or a wire protocol. The crate is not published
//! and makes no API-stability promise.
//!
//! # Ownership and thread safety
//!
//! Control and state values are copied into bounded host-owned storage. A
//! [`ResourceRegistry`] owns immutable bulk resources and returns
//! generation-checked handles; a [`ResourceView`] keeps the underlying bytes
//! alive even after the registry entry is released. A resource handle by
//! itself does not keep a resource alive.
//!
//! [`ControlQueue`] and [`StatePublisher`] are multi-thread-safe and use
//! locking or reference counting appropriate for ordinary application/UI
//! traffic. [`LatestTelemetry`] and [`SpscStream`] are intentionally narrower:
//! they are non-blocking, preallocated mechanisms for one producer and one
//! consumer. They may be suitable for a realtime-adjacent bridge, but this
//! crate is not an audio engine and does not claim that every operation is
//! suitable for a hard realtime callback. In particular, resource and state
//! publication allocate during publication and must not be called from such a
//! callback.
//!
//! A dropped notification is observable: control sends return a bounded queue
//! error, telemetry retains only its newest value, and stream pushes return
//! [`StreamError::Full`] while incrementing a dropped counter. No mechanism
//! silently grows its storage.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;

/// A monotonically increasing publication or resource generation number.
pub type Revision = u64;

/// Conservative defaults used by the individual mechanisms.
pub const DEFAULT_CONTROL_CAPACITY: usize = 256;
/// Default maximum size of one semantic control payload.
pub const DEFAULT_MAX_CONTROL_MESSAGE_BYTES: usize = 64 * 1024;
/// Default maximum state publication size.
pub const DEFAULT_MAX_STATE_BYTES: usize = 1024 * 1024;
/// Default maximum immutable resource size.
pub const DEFAULT_MAX_RESOURCE_BYTES: usize = 64 * 1024 * 1024;
/// Default maximum number of simultaneously retained registry entries.
pub const DEFAULT_MAX_RESOURCES: usize = 4096;
/// Default number of values in a telemetry sample.
pub const DEFAULT_TELEMETRY_WIDTH: usize = 8;
/// Default number of values a stream can retain.
pub const DEFAULT_STREAM_CAPACITY: usize = 1024;

/// Errors returned by bounded Caliber mechanisms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A requested capacity or width was zero or exceeded the safe bound.
    InvalidLimit {
        /// The name of the invalid limit.
        name: &'static str,
    },
    /// A payload exceeded the configured bound.
    TooLarge {
        /// Name of the bounded payload.
        kind: &'static str,
        /// Actual payload size.
        actual: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// A bounded queue or registry has no available capacity.
    Full {
        /// Name of the full mechanism.
        kind: &'static str,
    },
    /// A resource handle refers to a released or replaced generation.
    StaleResource(ResourceHandle),
    /// A resource handle refers to no known slot.
    UnknownResource(ResourceHandle),
    /// The caller supplied a buffer with the wrong fixed size.
    WrongSize {
        /// Name of the fixed-size value.
        kind: &'static str,
        /// Supplied size.
        actual: usize,
        /// Required size.
        expected: usize,
    },
    /// A state publication omitted its required non-zero schema version.
    InvalidSchemaVersion,
    /// No telemetry sample has been published yet.
    NoTelemetry,
    /// Another telemetry writer is currently publishing.
    TelemetryBusy,
    /// A revision would wrap and cannot remain monotonic.
    RevisionExhausted,
    /// A bounded allocation could not be reserved.
    AllocationFailed {
        /// Name of the allocation that failed.
        kind: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit { name } => write!(f, "invalid limit: {name}"),
            Self::TooLarge {
                kind,
                actual,
                maximum,
            } => write!(f, "{kind} is {actual} bytes, maximum is {maximum}"),
            Self::Full { kind } => write!(f, "{kind} is full"),
            Self::StaleResource(handle) => write!(f, "stale resource handle {handle:?}"),
            Self::UnknownResource(handle) => write!(f, "unknown resource handle {handle:?}"),
            Self::WrongSize {
                kind,
                actual,
                expected,
            } => write!(f, "{kind} has size {actual}, expected {expected}"),
            Self::InvalidSchemaVersion => f.write_str("state schema version must be non-zero"),
            Self::NoTelemetry => f.write_str("no telemetry value has been published"),
            Self::TelemetryBusy => f.write_str("telemetry is being published"),
            Self::RevisionExhausted => f.write_str("revision counter exhausted"),
            Self::AllocationFailed { kind } => write!(f, "allocation failed: {kind}"),
        }
    }
}

impl std::error::Error for Error {}

/// A bounded FIFO for semantic control payloads.
///
/// Each push copies one payload into queue-owned memory. Push is non-blocking:
/// it returns [`Error::Full`] when the queue contains `capacity` messages.
/// `try_pop` is safe to call from any consumer, but callers should normally
/// designate one consumer to preserve a simple application delivery order.
pub struct ControlQueue {
    inner: Mutex<VecDeque<Vec<u8>>>,
    capacity: usize,
    max_message_bytes: usize,
    wake: WakeSignal,
}

impl fmt::Debug for ControlQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ControlQueue")
            .field("capacity", &self.capacity)
            .field("max_message_bytes", &self.max_message_bytes)
            .field("len", &self.len())
            .finish()
    }
}

impl ControlQueue {
    /// Creates a bounded queue with the supplied message and count limits.
    pub fn new(capacity: usize, max_message_bytes: usize) -> Result<Self, Error> {
        if capacity == 0 {
            return Err(Error::InvalidLimit {
                name: "control capacity",
            });
        }
        if max_message_bytes == 0 {
            return Err(Error::InvalidLimit {
                name: "control message bytes",
            });
        }
        Ok(Self {
            inner: Mutex::new(VecDeque::new()),
            capacity,
            max_message_bytes,
            wake: WakeSignal::new(),
        })
    }

    /// Creates a queue with conservative defaults.
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_CONTROL_CAPACITY, DEFAULT_MAX_CONTROL_MESSAGE_BYTES)
            .expect("default control limits are valid")
    }

    /// Attempts to append a semantic payload in FIFO order.
    pub fn try_push(&self, payload: &[u8]) -> Result<(), Error> {
        if payload.len() > self.max_message_bytes {
            return Err(Error::TooLarge {
                kind: "control message",
                actual: payload.len(),
                maximum: self.max_message_bytes,
            });
        }
        let mut queue = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if queue.len() == self.capacity {
            return Err(Error::Full {
                kind: "control queue",
            });
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(payload.len())
            .map_err(|_| Error::AllocationFailed {
                kind: "control message",
            })?;
        owned.extend_from_slice(payload);
        queue.try_reserve(1).map_err(|_| Error::AllocationFailed {
            kind: "control queue",
        })?;
        queue.push_back(owned);
        drop(queue);
        self.wake.notify();
        Ok(())
    }

    /// Removes the oldest payload, or returns `None` when idle.
    pub fn try_pop(&self) -> Option<Vec<u8>> {
        let mut queue = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        queue.pop_front()
    }

    /// Waits for and removes the oldest payload.
    pub fn wait_pop(&self) -> Vec<u8> {
        loop {
            if let Some(payload) = self.try_pop() {
                return payload;
            }
            // A previous push may have left the level-triggered signal set
            // after the queue was drained. Clear that stale level only after
            // observing an empty queue, then recheck before parking so a push
            // cannot be lost between the check and the wait.
            self.wake.try_take();
            if !self.is_empty() {
                continue;
            }
            self.wake.wait();
        }
    }

    /// Returns the number of queued payloads.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .len()
    }

    /// Returns whether no payload is currently queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the queue's wake signal for an event-loop integration.
    pub fn wake_signal(&self) -> WakeSignal {
        self.wake.clone()
    }
}

/// One immutable, revisioned state publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePublication {
    revision: Revision,
    schema: u32,
    payload: Arc<[u8]>,
}

impl StatePublication {
    /// Returns the monotonic revision assigned at publication.
    pub fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the application-defined schema version for this publication.
    pub fn schema(&self) -> u32 {
        self.schema
    }

    /// Returns the immutable state bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Publishes whole state values atomically from a consumer's perspective.
///
/// A reader receives an [`Arc`] snapshot and therefore never observes a
/// partially replaced payload. Publication is application/UI work and may
/// allocate; it is not a realtime operation.
pub struct StatePublisher {
    current: RwLock<Option<Arc<StatePublication>>>,
    next_revision: AtomicU64,
    published_revision: AtomicU64,
    max_state_bytes: usize,
}

impl fmt::Debug for StatePublisher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StatePublisher")
            .field("max_state_bytes", &self.max_state_bytes)
            .field("latest_revision", &self.latest_revision())
            .finish()
    }
}

impl StatePublisher {
    /// Creates an empty publisher with a maximum payload size.
    pub fn new(max_state_bytes: usize) -> Result<Self, Error> {
        if max_state_bytes == 0 {
            return Err(Error::InvalidLimit {
                name: "state bytes",
            });
        }
        Ok(Self {
            current: RwLock::new(None),
            next_revision: AtomicU64::new(1),
            published_revision: AtomicU64::new(0),
            max_state_bytes,
        })
    }

    /// Creates a publisher with the default state bound.
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_MAX_STATE_BYTES).expect("default state limit is valid")
    }

    /// Atomically publishes a copied payload and returns its new revision.
    ///
    /// The core only requires a non-zero schema version; it does not interpret
    /// or validate the application-defined payload bytes.
    pub fn publish(&self, schema: u32, payload: &[u8]) -> Result<Revision, Error> {
        if schema == 0 {
            return Err(Error::InvalidSchemaVersion);
        }
        if payload.len() > self.max_state_bytes {
            return Err(Error::TooLarge {
                kind: "state publication",
                actual: payload.len(),
                maximum: self.max_state_bytes,
            });
        }
        // Serialize revision allocation with replacement. Otherwise two
        // producers could allocate revisions in order A/B but acquire the
        // publication lock in order B/A, making the visible revision go
        // backwards.
        let mut current = self
            .current
            .write()
            .unwrap_or_else(|poison| poison.into_inner());
        let revision = self
            .next_revision
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| Error::RevisionExhausted)?;
        let publication = Arc::new(StatePublication {
            revision,
            schema,
            payload: Arc::from(payload),
        });
        *current = Some(publication);
        self.published_revision.store(revision, Ordering::Release);
        Ok(revision)
    }

    /// Returns the current immutable publication, if one exists.
    pub fn read(&self) -> Option<Arc<StatePublication>> {
        self.current
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Returns the latest published revision without cloning the payload.
    pub fn latest_revision(&self) -> Option<Revision> {
        let revision = self.published_revision.load(Ordering::Acquire);
        (revision != 0).then_some(revision)
    }
}

/// Opaque identity for an immutable resource registry entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceHandle {
    id: u64,
    generation: Revision,
}

/// A registry-owned immutable resource view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceView {
    handle: ResourceHandle,
    bytes: Arc<[u8]>,
}

impl ResourceView {
    /// Returns the handle associated with this view.
    pub fn handle(&self) -> ResourceHandle {
        self.handle
    }

    /// Returns the immutable resource bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

struct ResourceSlot {
    generation: Revision,
    bytes: Option<Arc<[u8]>>,
}

/// Owns immutable bulk resources and validates generational handles.
///
/// Releasing a handle removes the registry's owning reference but does not
/// invalidate already-issued [`ResourceView`] values. New lookups using the
/// released handle fail, while views keep their immutable bytes alive until
/// their last owner drops them.
pub struct ResourceRegistry {
    slots: Mutex<HashMap<u64, ResourceSlot>>,
    next_id: AtomicU64,
    max_resources: usize,
    max_resource_bytes: usize,
}

impl fmt::Debug for ResourceRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResourceRegistry")
            .field("max_resources", &self.max_resources)
            .field("max_resource_bytes", &self.max_resource_bytes)
            .field("live_resources", &self.len())
            .finish()
    }
}

impl ResourceRegistry {
    /// Creates an empty registry with count and per-resource byte limits.
    pub fn new(max_resources: usize, max_resource_bytes: usize) -> Result<Self, Error> {
        if max_resources == 0 {
            return Err(Error::InvalidLimit {
                name: "resource count",
            });
        }
        if max_resource_bytes == 0 {
            return Err(Error::InvalidLimit {
                name: "resource bytes",
            });
        }
        Ok(Self {
            slots: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            max_resources,
            max_resource_bytes,
        })
    }

    /// Creates a registry using conservative defaults.
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_MAX_RESOURCES, DEFAULT_MAX_RESOURCE_BYTES)
            .expect("default resource limits are valid")
    }

    /// Inserts an immutable copy and returns its generation-checked handle.
    pub fn insert(&self, bytes: &[u8]) -> Result<ResourceHandle, Error> {
        if bytes.len() > self.max_resource_bytes {
            return Err(Error::TooLarge {
                kind: "resource",
                actual: bytes.len(),
                maximum: self.max_resource_bytes,
            });
        }
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if slots.values().filter(|slot| slot.bytes.is_some()).count() == self.max_resources {
            return Err(Error::Full {
                kind: "resource registry",
            });
        }
        let bytes = Arc::from(bytes);
        if let Some((&id, slot)) = slots.iter_mut().find(|(_, slot)| slot.bytes.is_none()) {
            let handle = ResourceHandle {
                id,
                generation: slot.generation,
            };
            slot.bytes = Some(bytes);
            return Ok(handle);
        }
        slots.try_reserve(1).map_err(|_| Error::AllocationFailed {
            kind: "resource registry",
        })?;
        let id = self
            .next_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| Error::RevisionExhausted)?;
        let handle = ResourceHandle { id, generation: 1 };
        slots.insert(
            id,
            ResourceSlot {
                generation: 1,
                bytes: Some(bytes),
            },
        );
        Ok(handle)
    }

    /// Resolves a live handle to an immutable view.
    pub fn get(&self, handle: ResourceHandle) -> Result<ResourceView, Error> {
        let slots = self
            .slots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(slot) = slots.get(&handle.id) else {
            return Err(Error::UnknownResource(handle));
        };
        if slot.generation != handle.generation {
            return Err(Error::StaleResource(handle));
        }
        let Some(bytes) = &slot.bytes else {
            return Err(Error::StaleResource(handle));
        };
        Ok(ResourceView {
            handle,
            bytes: bytes.clone(),
        })
    }

    /// Releases the registry's ownership of a live handle.
    pub fn release(&self, handle: ResourceHandle) -> Result<(), Error> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(slot) = slots.get_mut(&handle.id) else {
            return Err(Error::UnknownResource(handle));
        };
        if slot.generation != handle.generation || slot.bytes.is_none() {
            return Err(Error::StaleResource(handle));
        }
        slot.bytes = None;
        slot.generation = slot
            .generation
            .checked_add(1)
            .ok_or(Error::RevisionExhausted)?;
        Ok(())
    }

    /// Replaces a live resource in place and advances its generation.
    ///
    /// Existing [`ResourceView`] values retain the old immutable bytes. The
    /// returned handle is the only handle that resolves to the replacement.
    pub fn replace(&self, handle: ResourceHandle, bytes: &[u8]) -> Result<ResourceHandle, Error> {
        if bytes.len() > self.max_resource_bytes {
            return Err(Error::TooLarge {
                kind: "resource",
                actual: bytes.len(),
                maximum: self.max_resource_bytes,
            });
        }
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(slot) = slots.get_mut(&handle.id) else {
            return Err(Error::UnknownResource(handle));
        };
        if slot.generation != handle.generation || slot.bytes.is_none() {
            return Err(Error::StaleResource(handle));
        }
        let generation = slot
            .generation
            .checked_add(1)
            .ok_or(Error::RevisionExhausted)?;
        slot.generation = generation;
        slot.bytes = Some(Arc::from(bytes));
        Ok(ResourceHandle {
            id: handle.id,
            generation,
        })
    }

    /// Returns the number of live registry-owned resources.
    pub fn len(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .values()
            .filter(|slot| slot.bytes.is_some())
            .count()
    }

    /// Returns whether no registry-owned resources are live.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A fixed-width latest-value telemetry slot.
///
/// The slot uses a sequence lock over atomic bytes. A successful publish does
/// not allocate, block, or retain history. The intended contract is one
/// producer and any number of readers. An accidental concurrent producer gets
/// [`Error::TelemetryBusy`] rather than racing the active write.
pub struct LatestTelemetry {
    sequence: AtomicU64,
    bytes: Box<[AtomicUsize]>,
    width: usize,
}

impl fmt::Debug for LatestTelemetry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LatestTelemetry")
            .field("width", &self.width)
            .field("sequence", &self.sequence())
            .finish()
    }
}

impl LatestTelemetry {
    /// Creates a fixed-width telemetry slot. Width is measured in `usize`
    /// values, which keeps publication allocation-free and naturally aligned.
    pub fn new(width: usize) -> Result<Self, Error> {
        if width == 0 {
            return Err(Error::InvalidLimit {
                name: "telemetry width",
            });
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(width)
            .map_err(|_| Error::AllocationFailed {
                kind: "telemetry slot",
            })?;
        bytes.resize_with(width, || AtomicUsize::new(0));
        Ok(Self {
            sequence: AtomicU64::new(0),
            bytes: bytes.into_boxed_slice(),
            width,
        })
    }

    /// Creates a slot using the default width.
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_TELEMETRY_WIDTH).expect("default telemetry width is valid")
    }

    /// Publishes the newest fixed-width sample without allocating or blocking.
    pub fn publish(&self, values: &[usize]) -> Result<Revision, Error> {
        if values.len() != self.width {
            return Err(Error::WrongSize {
                kind: "telemetry sample",
                actual: values.len(),
                expected: self.width,
            });
        }
        let current = self.sequence.load(Ordering::Acquire);
        if current >= u64::MAX - 1 {
            return Err(Error::RevisionExhausted);
        }
        if !current.is_multiple_of(2)
            || self
                .sequence
                .compare_exchange(current, current + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
        {
            return Err(Error::TelemetryBusy);
        }
        for (slot, value) in self.bytes.iter().zip(values) {
            slot.store(*value, Ordering::Relaxed);
        }
        self.sequence.store(current + 2, Ordering::Release);
        Ok(current / 2 + 1)
    }

    /// Copies one coherent sample into a caller-owned fixed-size buffer.
    pub fn read_into(&self, output: &mut [usize]) -> Result<Revision, Error> {
        if output.len() != self.width {
            return Err(Error::WrongSize {
                kind: "telemetry output",
                actual: output.len(),
                expected: self.width,
            });
        }
        for _ in 0..32 {
            let before = self.sequence.load(Ordering::Acquire);
            if before == 0 {
                return Err(Error::NoTelemetry);
            }
            if !before.is_multiple_of(2) {
                std::hint::spin_loop();
                continue;
            }
            for (output, slot) in output.iter_mut().zip(&self.bytes) {
                *output = slot.load(Ordering::Relaxed);
            }
            let after = self.sequence.load(Ordering::Acquire);
            if before == after {
                return Ok(before / 2);
            }
            std::hint::spin_loop();
        }
        Err(Error::TelemetryBusy)
    }

    /// Returns the latest completed telemetry sequence, or zero before the
    /// first successful publication.
    pub fn sequence(&self) -> Revision {
        self.sequence.load(Ordering::Acquire) / 2
    }

    /// Returns the fixed number of values in each sample.
    pub fn width(&self) -> usize {
        self.width
    }
}

/// Errors specific to a bounded single-producer/single-consumer stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamError {
    /// The stream has no free slot; the value was not accepted.
    Full,
    /// The stream capacity is invalid.
    InvalidCapacity,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => f.write_str("SPSC stream is full"),
            Self::InvalidCapacity => f.write_str("SPSC stream capacity must be non-zero"),
        }
    }
}

impl std::error::Error for StreamError {}

struct StreamSlot<T> {
    value: std::cell::UnsafeCell<MaybeUninit<T>>,
}

/// A preallocated non-blocking SPSC ring buffer.
///
/// `T` is required to be `Copy` so dropping the stream cannot leak or double
/// drop values that are concurrently moving through slots. Exactly one thread
/// may call [`Self::push`] and exactly one thread may call [`Self::pop`].
/// `push` never allocates or blocks. A full push returns [`StreamError::Full`]
/// and increments [`Self::dropped_count`].
pub struct SpscStream<T: Copy + Send> {
    slots: Box<[StreamSlot<T>]>,
    capacity: usize,
    read: AtomicUsize,
    write: AtomicUsize,
    dropped: AtomicU64,
}

unsafe impl<T: Copy + Send> Send for SpscStream<T> {}
unsafe impl<T: Copy + Send> Sync for SpscStream<T> {}

impl<T: Copy + Send> fmt::Debug for SpscStream<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpscStream")
            .field("capacity", &self.capacity)
            .field("len", &self.len())
            .field("dropped", &self.dropped_count())
            .finish()
    }
}

impl<T: Copy + Send> SpscStream<T> {
    /// Allocates a stream with exactly `capacity` usable values.
    pub fn new(capacity: usize) -> Result<Self, StreamError> {
        let Some(slot_count) = capacity.checked_add(1) else {
            return Err(StreamError::InvalidCapacity);
        };
        if capacity == 0 {
            return Err(StreamError::InvalidCapacity);
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(slot_count)
            .map_err(|_| StreamError::InvalidCapacity)?;
        slots.resize_with(slot_count, || StreamSlot {
            value: std::cell::UnsafeCell::new(MaybeUninit::uninit()),
        });
        Ok(Self {
            slots: slots.into_boxed_slice(),
            capacity,
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        })
    }

    /// Creates a stream with the default capacity.
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_STREAM_CAPACITY).expect("default stream capacity is valid")
    }

    /// Attempts to append one value without blocking.
    pub fn push(&self, value: T) -> Result<(), StreamError> {
        let write = self.write.load(Ordering::Relaxed);
        let next = (write + 1) % self.slots.len();
        if next == self.read.load(Ordering::Acquire) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Err(StreamError::Full);
        }
        // SAFETY: the single producer owns this write index until it publishes
        // `next` with Release; the consumer only reads after Acquire.
        unsafe { (*self.slots[write].value.get()).write(value) };
        self.write.store(next, Ordering::Release);
        Ok(())
    }

    /// Removes the oldest value, or returns `None` when the stream is empty.
    pub fn pop(&self) -> Option<T> {
        let read = self.read.load(Ordering::Relaxed);
        if read == self.write.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: the producer published this slot with Release and does not
        // reuse it until the consumer publishes the next read index.
        let value = unsafe { (*self.slots[read].value.get()).assume_init_read() };
        self.read
            .store((read + 1) % self.slots.len(), Ordering::Release);
        Some(value)
    }

    /// Returns the approximate number of queued values.
    pub fn len(&self) -> usize {
        let read = self.read.load(Ordering::Acquire);
        let write = self.write.load(Ordering::Acquire);
        (write + self.slots.len() - read) % self.slots.len()
    }

    /// Returns whether no values are queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns how many pushes were rejected because the stream was full.
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }

    /// Returns the configured number of usable slots.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// A coalescing wake signal for event-loop integrations.
///
/// Notifications are level-triggered: multiple `notify` calls before a
/// `try_take` collapse into one pending signal. Waiting does not consume the
/// signal, which prevents a waiter from losing a notification between checking
/// a queue and parking. `try_take` is the explicit acknowledgement operation.
#[derive(Clone)]
pub struct WakeSignal {
    inner: Arc<WakeInner>,
}

struct WakeInner {
    pending: AtomicBool,
    wait_lock: Mutex<()>,
    wait_cv: Condvar,
}

impl fmt::Debug for WakeSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WakeSignal")
            .field("pending", &self.is_pending())
            .finish()
    }
}

impl WakeSignal {
    /// Creates a clear signal.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(WakeInner {
                pending: AtomicBool::new(false),
                wait_lock: Mutex::new(()),
                wait_cv: Condvar::new(),
            }),
        }
    }

    /// Marks the signal pending and wakes one waiter.
    pub fn notify(&self) {
        self.inner.pending.store(true, Ordering::Release);
        self.inner.wait_cv.notify_one();
    }

    /// Returns and clears the pending level.
    pub fn try_take(&self) -> bool {
        self.inner.pending.swap(false, Ordering::AcqRel)
    }

    /// Returns whether a notification is pending without consuming it.
    pub fn is_pending(&self) -> bool {
        self.inner.pending.load(Ordering::Acquire)
    }

    /// Waits until the signal becomes pending.
    pub fn wait(&self) {
        let mut guard = self
            .inner
            .wait_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        while !self.is_pending() {
            guard = self
                .inner
                .wait_cv
                .wait(guard)
                .unwrap_or_else(|poison| poison.into_inner());
        }
    }

    /// Waits up to `timeout` and reports whether the signal became pending.
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let mut guard = self
            .inner
            .wait_lock
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self.is_pending() {
            return true;
        }
        let (new_guard, result) = self
            .inner
            .wait_cv
            .wait_timeout_while(guard, timeout, |_| !self.is_pending())
            .unwrap_or_else(|poison| poison.into_inner());
        guard = new_guard;
        drop(guard);
        let _ = result;
        self.is_pending()
    }
}

impl Default for WakeSignal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    #[test]
    fn control_queue_is_bounded_and_ordered() {
        let queue = ControlQueue::new(2, 3).unwrap();
        assert_eq!(queue.try_push(b"one"), Ok(()));
        assert_eq!(queue.try_push(b"two"), Ok(()));
        assert_eq!(
            queue.try_push(b"four"),
            Err(Error::TooLarge {
                kind: "control message",
                actual: 4,
                maximum: 3,
            })
        );
        assert_eq!(queue.try_pop().as_deref(), Some(&b"one"[..]));
        assert_eq!(queue.try_pop().as_deref(), Some(&b"two"[..]));
        assert!(queue.try_pop().is_none());
        assert!(queue.wake_signal().try_take());
        assert!(!queue.wake_signal().is_pending());
    }

    #[test]
    fn oversized_state_is_rejected_without_revision_change() {
        let publisher = StatePublisher::new(2).unwrap();
        assert_eq!(publisher.latest_revision(), None);
        assert_eq!(
            publisher.publish(1, b"abc"),
            Err(Error::TooLarge {
                kind: "state publication",
                actual: 3,
                maximum: 2,
            })
        );
        assert_eq!(publisher.latest_revision(), None);
        assert_eq!(
            publisher.publish(0, b"ok"),
            Err(Error::InvalidSchemaVersion)
        );
        let revision = publisher.publish(1, b"ok").unwrap();
        assert_eq!(revision, 1);
        let state = publisher.read().unwrap();
        assert_eq!(state.schema(), 1);
        assert_eq!(state.payload(), b"ok");
    }

    #[test]
    fn state_publication_revision_and_payload_are_coherent() {
        let publisher = Arc::new(StatePublisher::new(64).unwrap());
        let writer = Arc::clone(&publisher);
        let handle = thread::spawn(move || {
            for value in 1..=2_000_u64 {
                let payload = value.to_le_bytes();
                writer.publish(1, &payload).unwrap();
            }
        });
        while !handle.is_finished() {
            if let Some(snapshot) = publisher.read() {
                let value = u64::from_le_bytes(snapshot.payload().try_into().unwrap());
                assert!(snapshot.revision() >= 1);
                assert!(value >= 1);
            }
            thread::yield_now();
        }
        handle.join().unwrap();
        assert_eq!(
            publisher.read().unwrap().payload(),
            &2_000_u64.to_le_bytes()
        );
    }

    #[test]
    fn stale_resource_handles_are_rejected_but_views_keep_bytes_alive() {
        let registry = ResourceRegistry::new(1, 16).unwrap();
        let handle = registry.insert(b"immutable").unwrap();
        let view = registry.get(handle).unwrap();
        let replacement = registry.replace(handle, b"replacement").unwrap();
        assert_eq!(view.bytes(), b"immutable");
        assert_eq!(registry.get(handle), Err(Error::StaleResource(handle)));
        assert_eq!(registry.get(replacement).unwrap().bytes(), b"replacement");
        registry.release(replacement).unwrap();
        assert_eq!(
            registry.get(replacement),
            Err(Error::StaleResource(replacement))
        );
        assert_eq!(registry.len(), 0);
        let reused = registry.insert(b"reused").unwrap();
        assert_eq!(reused.id, replacement.id);
        assert!(reused.generation > replacement.generation);
        assert_eq!(registry.get(reused).unwrap().bytes(), b"reused");
    }

    #[test]
    fn telemetry_reads_are_coherent_and_publish_does_not_need_history() {
        let telemetry = Arc::new(LatestTelemetry::new(2).unwrap());
        assert_eq!(telemetry.read_into(&mut [0; 2]), Err(Error::NoTelemetry));
        let writer = Arc::clone(&telemetry);
        let finished = Arc::new(AtomicBool::new(false));
        let writer_finished = Arc::clone(&finished);
        let thread = thread::spawn(move || {
            for value in 1..=10_000_usize {
                writer.publish(&[value, !value]).unwrap();
            }
            writer_finished.store(true, Ordering::Release);
        });
        let mut output = [0; 2];
        while !finished.load(Ordering::Acquire) {
            if telemetry.read_into(&mut output).is_ok() {
                assert_eq!(output[1], !output[0]);
            }
            thread::yield_now();
        }
        thread.join().unwrap();
        telemetry.read_into(&mut output).unwrap();
        assert_eq!(output[1], !output[0]);
        assert_eq!(telemetry.sequence(), 10_000);
    }

    #[test]
    fn telemetry_rejects_wrong_width_and_concurrent_writer() {
        let telemetry = LatestTelemetry::new(2).unwrap();
        assert_eq!(
            telemetry.publish(&[1]),
            Err(Error::WrongSize {
                kind: "telemetry sample",
                actual: 1,
                expected: 2,
            })
        );
        // A completed publish leaves the slot available for the next writer.
        telemetry.publish(&[1, 2]).unwrap();
        assert_eq!(telemetry.sequence(), 1);
    }

    #[test]
    fn stream_reports_explicit_overflow_and_preserves_order() {
        let stream = SpscStream::new(2).unwrap();
        assert_eq!(stream.push(10), Ok(()));
        assert_eq!(stream.push(20), Ok(()));
        assert_eq!(stream.push(30), Err(StreamError::Full));
        assert_eq!(stream.dropped_count(), 1);
        assert_eq!(stream.pop(), Some(10));
        assert_eq!(stream.pop(), Some(20));
        assert_eq!(stream.pop(), None);
    }

    #[test]
    fn stream_rejects_capacities_that_cannot_add_ring_sentinel() {
        assert!(matches!(
            SpscStream::<u8>::new(usize::MAX),
            Err(StreamError::InvalidCapacity)
        ));
        assert!(matches!(
            SpscStream::<u8>::new(usize::MAX - 1),
            Err(StreamError::InvalidCapacity)
        ));
    }

    #[test]
    fn stream_handles_one_producer_and_one_consumer() {
        let stream = Arc::new(SpscStream::new(32).unwrap());
        let producer_stream = Arc::clone(&stream);
        let producer = thread::spawn(move || {
            for value in 0..10_000_u64 {
                while producer_stream.push(value).is_err() {
                    thread::yield_now();
                }
            }
        });
        let mut next = 0;
        while next < 10_000 {
            if let Some(value) = stream.pop() {
                assert_eq!(value, next);
                next += 1;
            } else {
                thread::yield_now();
            }
        }
        producer.join().unwrap();
    }

    #[test]
    fn wake_signal_is_idle_until_notified_and_coalesces_notifications() {
        let signal = WakeSignal::new();
        assert!(!signal.is_pending());
        assert!(!signal.wait_timeout(Duration::from_millis(1)));
        signal.notify();
        signal.notify();
        assert!(signal.is_pending());
        assert!(signal.wait_timeout(Duration::from_millis(1)));
        assert!(signal.try_take());
        assert!(!signal.try_take());
    }

    #[test]
    fn waiting_control_queue_does_not_poll_while_idle() {
        let queue = Arc::new(ControlQueue::new(1, 8).unwrap());
        let producer_queue = Arc::clone(&queue);
        let consumer = thread::spawn(move || {
            let first = producer_queue.wait_pop();
            let second = producer_queue.wait_pop();
            (first, second)
        });
        thread::sleep(Duration::from_millis(2));
        queue.try_push(b"ready").unwrap();
        thread::sleep(Duration::from_millis(2));
        queue.try_push(b"again").unwrap();
        assert_eq!(
            consumer.join().unwrap(),
            (b"ready".to_vec(), b"again".to_vec())
        );
    }
}
