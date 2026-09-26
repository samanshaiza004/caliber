use super::{
    Error, ProjectConfig, Result, SyncState, canonical_project_root, read_lock, status_project,
};
use caliber_ffi::{
    CALIBER_ABI_VERSION_1, CaliberApiV1, CaliberContext, CaliberResourceView,
    CaliberStatePublication, CaliberStatus,
};
use std::collections::BTreeSet;
use std::env;
#[cfg(unix)]
use std::ffi::c_int;
use std::ffi::{CStr, CString, c_char, c_void};
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::ptr;
use std::slice;
use std::sync::mpsc::sync_channel;
use std::thread;

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DiagnosticsConfig {
    #[serde(default)]
    required_tools: Vec<String>,
    #[serde(default)]
    platform_tools: PlatformTools,
    #[serde(default)]
    caliber_library: PlatformPaths,
    #[serde(default = "default_requested_abi")]
    requested_abi: u32,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformTools {
    #[serde(default)]
    windows: Vec<String>,
    #[serde(default)]
    macos: Vec<String>,
    #[serde(default)]
    linux: Vec<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformPaths {
    windows: Option<PathBuf>,
    macos: Option<PathBuf>,
    linux: Option<PathBuf>,
}

fn default_requested_abi() -> u32 {
    CALIBER_ABI_VERSION_1
}

#[derive(Debug)]
struct ApiSummary {
    version: u32,
    table_size: u32,
}

pub(super) fn doctor(root: &Path, library_override: Option<&Path>) -> Result<()> {
    inspect_project(root, library_override).map(|_| ())
}

pub(super) fn check(root: &Path, library_override: Option<&Path>) -> Result<()> {
    let library_path = inspect_project(root, library_override)?;
    let config = read_project_config(root)?;
    let diagnostics = config
        .diagnostics
        .ok_or_else(|| Error::new("caliber.config.json has no diagnostics configuration"))?;
    println!("Caliber boundary check");
    with_loaded_api(&library_path, diagnostics.requested_abi, run_boundary_smoke)?;
    println!("  result: passed");
    Ok(())
}

fn inspect_project(root: &Path, library_override: Option<&Path>) -> Result<PathBuf> {
    let root = canonical_project_root(root)?;
    let mut failures = Vec::new();
    println!("Project");
    println!("  root: {}", root.display());

    match read_lock(&root) {
        Ok(_) => {
            println!("  lock: valid");
            match status_project(&root) {
                Ok(entries) => {
                    let synchronized = entries
                        .iter()
                        .filter(|entry| entry.state == SyncState::Synchronized)
                        .count();
                    println!(
                        "  managed dependencies: {synchronized}/{} synchronized",
                        entries.len()
                    );
                    for entry in entries {
                        let state = match entry.state {
                            SyncState::Synchronized => "synchronized",
                            SyncState::Missing => "missing",
                            SyncState::Mismatched => "mismatched",
                        };
                        let working_tree = match entry.dirty {
                            Some(true) => "dirty",
                            Some(false) => "clean",
                            None => "unknown",
                        };
                        println!(
                            "    {}: {state}; working tree={working_tree}; locked={} HEAD={} path={}",
                            entry.name,
                            entry.revision,
                            entry.head.as_deref().unwrap_or("-"),
                            entry.path.display()
                        );
                        if let Some(detail) = &entry.detail {
                            println!("      detail: {detail}");
                        }
                        if entry.state != SyncState::Synchronized {
                            failures.push(format!(
                                "dependency {} is {state}; run caliber sync after reviewing the managed checkout",
                                entry.name
                            ));
                        }
                    }
                }
                Err(error) => {
                    println!("  managed dependencies: unavailable ({error})");
                    failures.push(error.to_string());
                }
            }
        }
        Err(error) => {
            println!("  lock: invalid ({error})");
            failures.push(error.to_string());
        }
    }

    println!("Host");
    println!("  platform: {}/{}", env::consts::OS, env::consts::ARCH);
    let config = match read_project_config(&root) {
        Ok(config) => config,
        Err(error) => {
            println!("  project config: invalid ({error})");
            failures.push(error.to_string());
            return finish_report(None, failures);
        }
    };
    let Some(diagnostics) = config.diagnostics else {
        let message = "caliber.config.json has no diagnostics configuration";
        println!("  project diagnostics: not configured");
        failures.push(message.to_owned());
        return finish_report(None, failures);
    };

    println!("  required tools:");
    let mut required_tools = diagnostics.required_tools.clone();
    let platform_tools: &[String] = match env::consts::OS {
        "windows" => &diagnostics.platform_tools.windows,
        "macos" => &diagnostics.platform_tools.macos,
        "linux" => &diagnostics.platform_tools.linux,
        _ => &[],
    };
    required_tools.extend(platform_tools.iter().cloned());
    let mut unique_tools = BTreeSet::new();
    for tool in required_tools {
        if tool.trim().is_empty() || !unique_tools.insert(tool.clone()) {
            if tool.trim().is_empty() {
                failures.push("diagnostics configuration contains an empty tool name".into());
            }
            continue;
        }
        match find_executable(&tool) {
            Some(path) => println!("    {tool}: found at {}", path.display()),
            None => {
                println!("    {tool}: missing");
                failures.push(format!("required tool {tool:?} was not found on PATH"));
            }
        }
    }

    let library_path = match library_override {
        Some(path) => Some(resolve_project_path(&root, path)),
        None => configured_library_path(&root, &diagnostics.caliber_library),
    };
    println!("Caliber library");
    let Some(library_path) = library_path else {
        println!("  path: not configured for {}", env::consts::OS);
        failures.push(format!(
            "no Caliber library path is configured for {}",
            env::consts::OS
        ));
        return finish_report(None, failures);
    };
    println!("  path: {}", library_path.display());
    if !library_path.is_file() {
        println!("  loadable: no (file is missing)");
        failures.push(format!(
            "Caliber library is missing at {}; build the project target that produces it",
            library_path.display()
        ));
        return finish_report(None, failures);
    }
    match inspect_library(&library_path, diagnostics.requested_abi) {
        Ok(summary) => {
            println!("  architecture: compatible with host {}", env::consts::ARCH);
            println!("  loadable: yes");
            println!("ABI");
            println!("  requested version: {}", diagnostics.requested_abi);
            println!("  reported version: {}", summary.version);
            println!(
                "  table extent: {} bytes (required {} bytes)",
                summary.table_size,
                mem::size_of::<CaliberApiV1>()
            );
            println!("  required functions: present");
        }
        Err(error) => {
            println!("  loadable/ABI check: failed ({error})");
            failures.push(error.to_string());
        }
    }
    finish_report(Some(library_path), failures)
}

fn finish_report(path: Option<PathBuf>, failures: Vec<String>) -> Result<PathBuf> {
    if failures.is_empty() {
        println!("  result: ready");
        path.ok_or_else(|| Error::new("Caliber library path was not resolved"))
    } else {
        let mut message = format!("doctor found {} issue(s):", failures.len());
        for failure in failures {
            message.push_str("\n  - ");
            message.push_str(&failure);
        }
        Err(Error::new(message))
    }
}

fn read_project_config(root: &Path) -> Result<ProjectConfig> {
    let path = root.join("caliber.config.json");
    let bytes = fs::read(&path)
        .map_err(|error| Error::new(format!("cannot read {}: {error}", path.display())))?;
    let config: ProjectConfig = serde_json::from_slice(&bytes)
        .map_err(|error| Error::new(format!("malformed {}: {error}", path.display())))?;
    if config.schema != 1 {
        return Err(Error::new(format!(
            "malformed project config: unsupported schema {}",
            config.schema
        )));
    }
    Ok(config)
}

fn configured_library_path(root: &Path, paths: &PlatformPaths) -> Option<PathBuf> {
    let configured = match env::consts::OS {
        "windows" => paths.windows.as_ref(),
        "macos" => paths.macos.as_ref(),
        "linux" => paths.linux.as_ref(),
        _ => None,
    }?;
    Some(resolve_project_path(root, configured))
}

fn resolve_project_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

fn find_executable(program: &str) -> Option<PathBuf> {
    let requested = Path::new(program);
    let explicit_path = requested.is_absolute() || requested.components().count() > 1;
    if explicit_path {
        return requested.is_file().then(|| requested.to_owned());
    }
    let search_paths = env::var_os("PATH")?;
    #[cfg(windows)]
    let extensions: Vec<String> = {
        let configured = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        std::iter::once(String::new())
            .chain(configured.split(';').map(str::to_owned))
            .collect()
    };
    #[cfg(not(windows))]
    let extensions = vec![String::new()];
    for directory in env::split_paths(&search_paths) {
        for extension in &extensions {
            let mut candidate = directory.join(program);
            if !extension.is_empty() && candidate.extension().is_none() {
                candidate.set_extension(extension.trim_start_matches('.'));
            }
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn inspect_library(path: &Path, requested_abi: u32) -> Result<ApiSummary> {
    with_loaded_api(path, requested_abi, |api| {
        Ok(ApiSummary {
            version: api.abi_version,
            table_size: api.struct_size,
        })
    })
}

fn with_loaded_api<T>(
    path: &Path,
    requested_abi: u32,
    use_api: impl FnOnce(&CaliberApiV1) -> Result<T>,
) -> Result<T> {
    if requested_abi != CALIBER_ABI_VERSION_1 {
        return Err(Error::new(format!(
            "this Caliber CLI supports ABI version {CALIBER_ABI_VERSION_1}, not requested version {requested_abi}"
        )));
    }
    let library = DynamicLibrary::open(path)
        .map_err(|error| Error::new(format!("load {}: {error}", path.display())))?;
    let symbol = CString::new("caliber_get_api").expect("static symbol name");
    let symbol = library
        .symbol(&symbol)
        .map_err(|error| Error::new(format!("{}: {error}", path.display())))?;
    // SAFETY: caliber_get_api is the documented C entry point. The loaded
    // library remains alive for the entire call and every function use below.
    let get_api: unsafe extern "C" fn(u32) -> *const CaliberApiV1 =
        unsafe { mem::transmute(symbol) };
    // SAFETY: the requested ABI version is checked above and the library is live.
    let api_ptr = unsafe { get_api(requested_abi) };
    if api_ptr.is_null() {
        return Err(Error::new(format!(
            "{} does not provide requested ABI version {requested_abi}",
            path.display()
        )));
    }
    // The ABI guarantees its first two fields are u32 header values. Check the
    // advertised extent before reading any later function pointer.
    let header = unsafe { &*api_ptr.cast::<ApiHeader>() };
    if header.struct_size < mem::size_of::<ApiHeader>() as u32 {
        return Err(Error::new(format!(
            "ABI table header is too small: reported {} bytes, required {} bytes",
            header.struct_size,
            mem::size_of::<ApiHeader>()
        )));
    }
    if header.abi_version != requested_abi {
        return Err(Error::new(format!(
            "ABI version mismatch: requested {requested_abi}, library reported {}",
            header.abi_version
        )));
    }
    let required_size = mem::size_of::<CaliberApiV1>();
    if (header.struct_size as usize) < required_size {
        return Err(Error::new(format!(
            "ABI table is too small: reported {} bytes, required {required_size} bytes for wake and resource functions; rebuild the Caliber FFI library",
            header.struct_size
        )));
    }
    // SAFETY: the reported table extent covers the complete v1 table.
    let api = unsafe { &*api_ptr };
    let missing = missing_functions(api);
    if !missing.is_empty() {
        return Err(Error::new(format!(
            "ABI table is missing required functions: {}",
            missing.join(", ")
        )));
    }
    use_api(api)
}

#[repr(C)]
struct ApiHeader {
    abi_version: u32,
    struct_size: u32,
}

fn missing_functions(api: &CaliberApiV1) -> Vec<&'static str> {
    let mut missing = Vec::new();
    macro_rules! required {
        ($($field:ident),+ $(,)?) => {
            $(if api.$field.is_none() { missing.push(stringify!($field)); })+
        };
    }
    required!(
        context_create,
        context_destroy,
        context_dispatch,
        context_peek_command,
        context_take_command,
        context_publish_state,
        context_read_latest_state,
        state_publication_release,
        context_map_resource,
        resource_release,
        context_publish_resource,
        context_release_resource,
        context_wake_sequence,
        context_wait_wake,
        context_stop_wake_waiters,
    );
    missing
}

fn run_boundary_smoke(api: &CaliberApiV1) -> Result<()> {
    let create = api.context_create.expect("validated function table");
    let mut raw_context: *mut CaliberContext = ptr::null_mut();
    // SAFETY: null selects the documented default configuration, and the
    // output pointer addresses valid caller-owned storage.
    let status = unsafe { create(ptr::null(), &mut raw_context) };
    expect_status("create context", status, CaliberStatus::Ok)?;
    if raw_context.is_null() {
        return Err(Error::new(
            "context_create returned success with a null handle",
        ));
    }
    let mut context = ContextGuard {
        api,
        context: raw_context,
    };
    println!("  context create: passed");

    let command = b"caliber-check-command";
    let dispatch = api.context_dispatch.expect("validated function table");
    // SAFETY: the handle is live and the byte slice is valid for this call.
    expect_status(
        "dispatch command",
        unsafe { dispatch(context.context, command.as_ptr(), command.len()) },
        CaliberStatus::Ok,
    )?;
    let peek = api.context_peek_command.expect("validated function table");
    let take = api.context_take_command.expect("validated function table");
    let mut command_size = 0usize;
    // SAFETY: the context is live and command_size is valid output storage.
    expect_status(
        "peek command",
        unsafe { peek(context.context, &mut command_size) },
        CaliberStatus::Ok,
    )?;
    if command_size != command.len() {
        return Err(Error::new(format!(
            "command size mismatch: expected {}, received {command_size}",
            command.len()
        )));
    }
    let mut command_copy = vec![0u8; command_size];
    let mut copied = 0usize;
    // SAFETY: the buffer is caller-owned and has the exact reported capacity.
    expect_status(
        "take command",
        unsafe {
            take(
                context.context,
                command_copy.as_mut_ptr(),
                command_copy.len(),
                &mut copied,
            )
        },
        CaliberStatus::Ok,
    )?;
    if copied != command.len() || command_copy != command {
        return Err(Error::new(
            "dispatch/read command payload did not round-trip",
        ));
    }
    println!("  command dispatch/read: passed");

    let state_payload = b"caliber-check-state";
    let publish_state = api.context_publish_state.expect("validated function table");
    let mut state_revision = 0u64;
    // SAFETY: the context/payload are valid and revision is writable output.
    expect_status(
        "publish state",
        unsafe {
            publish_state(
                context.context,
                0x43484B31,
                state_payload.as_ptr(),
                state_payload.len(),
                &mut state_revision,
            )
        },
        CaliberStatus::Ok,
    )?;
    let read_state = api
        .context_read_latest_state
        .expect("validated function table");
    let release_state = api
        .state_publication_release
        .expect("validated function table");
    let mut state = CaliberStatePublication::default();
    // SAFETY: the context is live and state is writable output storage.
    let read_status = unsafe { read_state(context.context, &mut state) };
    if read_status != CaliberStatus::Ok {
        return Err(Error::new(format!("read state returned {read_status:?}")));
    }
    // SAFETY: publication bytes remain leased until release_state below.
    let state_matches = state.revision == state_revision
        && state.schema == 0x43484B31
        && !state.data.is_null()
        && unsafe { slice::from_raw_parts(state.data, state.len) } == state_payload;
    // SAFETY: state was filled by the matching ABI read function.
    unsafe { release_state(&mut state) };
    if !state_matches {
        return Err(Error::new("state publication did not round-trip"));
    }
    println!("  state publish/read/release: passed");

    let resource_payload = b"caliber-check-resource";
    let publish_resource = api
        .context_publish_resource
        .expect("validated function table");
    let mut resource_id = 0u64;
    let mut generation = 0u64;
    // SAFETY: payload and context are valid; IDs are writable output storage.
    expect_status(
        "publish resource",
        unsafe {
            publish_resource(
                context.context,
                resource_payload.as_ptr(),
                resource_payload.len(),
                &mut resource_id,
                &mut generation,
            )
        },
        CaliberStatus::Ok,
    )?;
    let map_resource = api.context_map_resource.expect("validated function table");
    let release_view = api.resource_release.expect("validated function table");
    let release_resource = api
        .context_release_resource
        .expect("validated function table");
    let mut view = CaliberResourceView::default();
    // SAFETY: the context is live and view is writable output storage.
    let map_status = unsafe { map_resource(context.context, resource_id, generation, &mut view) };
    if map_status != CaliberStatus::Ok {
        return Err(Error::new(format!("map resource returned {map_status:?}")));
    }
    // SAFETY: bytes remain leased until release_view below.
    let resource_matches = !view.data.is_null()
        && unsafe { slice::from_raw_parts(view.data, view.len) } == resource_payload;
    // SAFETY: view was filled by the matching ABI map function.
    unsafe { release_view(&mut view) };
    // SAFETY: this context owns the published resource handle.
    expect_status(
        "release resource owner",
        unsafe { release_resource(context.context, resource_id, generation) },
        CaliberStatus::Ok,
    )?;
    if !resource_matches {
        return Err(Error::new("mapped resource did not round-trip"));
    }
    println!("  resource publish/map/release: passed");

    let wake_sequence = api.context_wake_sequence.expect("validated function table");
    let wait_wake = api.context_wait_wake.expect("validated function table");
    let stop_waiters = api
        .context_stop_wake_waiters
        .expect("validated function table");
    let mut observed = 0u64;
    // SAFETY: observed is writable output storage and context is live.
    expect_status(
        "read wake sequence",
        unsafe { wake_sequence(context.context, &mut observed) },
        CaliberStatus::Ok,
    )?;
    let context_address = context.context as usize;
    let (started_tx, started_rx) = sync_channel(0);
    let waiter = thread::spawn(move || {
        let _ = started_tx.send(());
        let mut next = 0u64;
        // SAFETY: context remains live until this joined waiter returns.
        let status = unsafe {
            wait_wake(
                context_address as *const CaliberContext,
                observed,
                &mut next,
            )
        };
        (status, next)
    });
    started_rx
        .recv()
        .map_err(|error| Error::new(format!("start wake waiter: {error}")))?;
    // Dispatching a command advances the sequence and wakes the waiter.
    // SAFETY: context and command bytes are valid.
    let wake_dispatch_status =
        unsafe { dispatch(context.context, command.as_ptr(), command.len()) };
    if wake_dispatch_status != CaliberStatus::Ok {
        // SAFETY: stop releases the worker if dispatch failed before waking it.
        let _ = unsafe { stop_waiters(context.context) };
    }
    let (wait_status, next_sequence) = waiter
        .join()
        .map_err(|_| Error::new("wake waiter thread panicked"))?;
    expect_status(
        "dispatch wake command",
        wake_dispatch_status,
        CaliberStatus::Ok,
    )?;
    expect_status("wait for wake", wait_status, CaliberStatus::Ok)?;
    if next_sequence == observed {
        return Err(Error::new("wake wait returned without a sequence change"));
    }

    let (stopped_tx, stopped_rx) = sync_channel(0);
    let stopped_context_address = context.context as usize;
    let stopped_waiter = thread::spawn(move || {
        let _ = stopped_tx.send(());
        let mut sequence = 0u64;
        // SAFETY: context remains live until this joined waiter returns.
        unsafe {
            wait_wake(
                stopped_context_address as *const CaliberContext,
                next_sequence,
                &mut sequence,
            )
        }
    });
    stopped_rx
        .recv()
        .map_err(|error| Error::new(format!("start stoppable wake waiter: {error}")))?;
    // SAFETY: stop wakes an active waiter and is idempotent.
    let stop_status = unsafe { stop_waiters(context.context) };
    if stop_status != CaliberStatus::Ok {
        // SAFETY: second stop prevents leaving a blocking worker behind.
        let _ = unsafe { stop_waiters(context.context) };
    }
    let stopped_status = stopped_waiter
        .join()
        .map_err(|_| Error::new("stoppable wake waiter thread panicked"))?;
    expect_status("stop wake waiters", stop_status, CaliberStatus::Ok)?;
    expect_status(
        "wake waiter shutdown",
        stopped_status,
        CaliberStatus::Stopped,
    )?;
    println!("  wake sequence/wait/stop/join: passed");

    context.destroy();
    println!("  context destroy: passed");
    Ok(())
}

fn expect_status(operation: &str, actual: CaliberStatus, expected: CaliberStatus) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::new(format!(
            "{operation} returned {actual:?}, expected {expected:?}"
        )))
    }
}

struct ContextGuard<'a> {
    api: &'a CaliberApiV1,
    context: *mut CaliberContext,
}

impl ContextGuard<'_> {
    fn destroy(&mut self) {
        if self.context.is_null() {
            return;
        }
        if let Some(stop) = self.api.context_stop_wake_waiters {
            // SAFETY: guard owns the live context and all waiters are joined.
            let _ = unsafe { stop(self.context) };
        }
        if let Some(destroy) = self.api.context_destroy {
            // SAFETY: guard owns the context and no operation is active.
            unsafe { destroy(self.context) };
        }
        self.context = ptr::null_mut();
    }
}

impl Drop for ContextGuard<'_> {
    fn drop(&mut self) {
        self.destroy();
    }
}

#[cfg(windows)]
struct DynamicLibrary(*mut c_void);

#[cfg(unix)]
struct DynamicLibrary(*mut c_void);

#[cfg(windows)]
impl DynamicLibrary {
    fn open(path: &Path) -> std::result::Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        // SAFETY: wide is a nul-terminated UTF-16 path.
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            // SAFETY: GetLastError has no preconditions.
            return Err(format!("Windows loader error {}", unsafe {
                GetLastError()
            }));
        }
        Ok(Self(handle))
    }

    fn symbol(&self, name: &CStr) -> std::result::Result<*mut c_void, String> {
        // SAFETY: module is live and name is nul-terminated.
        let symbol = unsafe { GetProcAddress(self.0, name.as_ptr()) };
        if symbol.is_null() {
            // SAFETY: GetLastError has no preconditions.
            return Err(format!("GetProcAddress failed with {}", unsafe {
                GetLastError()
            }));
        }
        Ok(symbol)
    }
}

#[cfg(windows)]
impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: handle was returned by LoadLibraryW and is released once.
            unsafe { FreeLibrary(self.0) };
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(path: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
    fn GetLastError() -> u32;
}

#[cfg(unix)]
impl DynamicLibrary {
    fn open(path: &Path) -> std::result::Result<Self, String> {
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| "library path contains a NUL byte".to_owned())?;
        // SAFETY: path is nul-terminated; 2 requests immediate symbol binding.
        let handle = unsafe { dlopen(path.as_ptr(), 2) };
        if handle.is_null() {
            return Err(loader_error());
        }
        Ok(Self(handle))
    }

    fn symbol(&self, name: &CStr) -> std::result::Result<*mut c_void, String> {
        // SAFETY: dlerror has no preconditions.
        unsafe { dlerror() };
        // SAFETY: module is live and name is nul-terminated.
        let symbol = unsafe { dlsym(self.0, name.as_ptr()) };
        // SAFETY: dlerror has no preconditions.
        let error = unsafe { dlerror() };
        if !error.is_null() {
            return Err(unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned());
        }
        if symbol.is_null() {
            return Err("symbol address is null".into());
        }
        Ok(symbol)
    }
}

#[cfg(unix)]
impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: handle was returned by dlopen and is released once.
            unsafe { dlclose(self.0) };
        }
    }
}

#[cfg(target_os = "linux")]
#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

#[cfg(unix)]
fn loader_error() -> String {
    // SAFETY: dlerror returns a thread-local string or null.
    let error = unsafe { dlerror() };
    if error.is_null() {
        "dynamic loader returned no diagnostic".into()
    } else {
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CALIBER_ABI_VERSION_1, DiagnosticsConfig, configured_library_path, find_executable,
        missing_functions,
    };
    use caliber_ffi::CaliberApiV1;
    use std::mem;
    use std::path::Path;

    #[test]
    fn diagnostics_config_defaults_to_current_abi() {
        let config: DiagnosticsConfig = serde_json::from_str(
            r#"{"required_tools":["git"],"caliber_library":{"windows":"out/caliber_ffi.dll"}}"#,
        )
        .unwrap();
        assert_eq!(config.requested_abi, 1);
        assert_eq!(config.required_tools, ["git"]);
    }

    #[test]
    fn finds_an_explicit_executable_path_without_mutating_it() {
        let executable = std::env::current_exe().unwrap();
        assert_eq!(
            find_executable(executable.to_str().unwrap()),
            Some(executable)
        );
        assert_eq!(find_executable("definitely-not-a-caliber-tool"), None);
    }

    #[test]
    fn linked_abi_table_contains_every_function_required_by_check() {
        let api_ptr = caliber_ffi::caliber_get_api(CALIBER_ABI_VERSION_1);
        assert!(!api_ptr.is_null());
        // SAFETY: the ABI function returns a static table for this version.
        let api = unsafe { &*api_ptr };
        assert!(missing_functions(api).is_empty());
        assert!(api.struct_size as usize >= mem::size_of::<CaliberApiV1>());
    }

    #[test]
    fn resolves_platform_library_paths_under_project_root() {
        let paths: super::PlatformPaths = serde_json::from_str(
            r#"{"windows":"out/a.dll","macos":"out/a.dylib","linux":"out/a.so"}"#,
        )
        .unwrap();
        let root = Path::new("C:/project");
        #[cfg(windows)]
        assert!(
            configured_library_path(root, &paths)
                .unwrap()
                .ends_with("out/a.dll")
        );
        #[cfg(target_os = "macos")]
        assert!(
            configured_library_path(root, &paths)
                .unwrap()
                .ends_with("out/a.dylib")
        );
        #[cfg(target_os = "linux")]
        assert!(
            configured_library_path(root, &paths)
                .unwrap()
                .ends_with("out/a.so")
        );
    }
}
