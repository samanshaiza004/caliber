use std::ffi::{CString, c_char, c_int};
use std::mem::{offset_of, size_of};
use std::path::PathBuf;

use caliber_ffi::{
    CaliberApiV1, CaliberContextConfig, CaliberResourceView, CaliberStatePublication,
    CaliberStatus, CaliberTelemetryInfo,
};

#[link(name = "caliber_abi_clients", kind = "static")]
unsafe extern "C" {
    fn caliber_abi_c_layout(out: *mut usize);
    fn caliber_old_v1_client_run(library_path: *const c_char) -> c_int;
    fn caliber_current_v1_client_run(library_path: *const c_char) -> c_int;
}

#[test]
fn canonical_c_header_layout_matches_rust_implementation() {
    let mut actual = [0_usize; 48];
    // SAFETY: the C test helper writes exactly 48 size/offset entries.
    unsafe { caliber_abi_c_layout(actual.as_mut_ptr()) };

    let expected = [
        size_of::<CaliberStatus>(),
        size_of::<CaliberContextConfig>(),
        offset_of!(CaliberContextConfig, struct_size),
        offset_of!(CaliberContextConfig, max_command_bytes),
        offset_of!(CaliberContextConfig, max_publication_bytes),
        offset_of!(CaliberContextConfig, max_resource_bytes),
        offset_of!(CaliberContextConfig, max_resources),
        offset_of!(CaliberContextConfig, telemetry_width),
        offset_of!(CaliberContextConfig, max_pending_commands),
        size_of::<CaliberStatePublication>(),
        offset_of!(CaliberStatePublication, revision),
        offset_of!(CaliberStatePublication, schema),
        offset_of!(CaliberStatePublication, reserved),
        offset_of!(CaliberStatePublication, data),
        offset_of!(CaliberStatePublication, len),
        offset_of!(CaliberStatePublication, lease),
        size_of::<CaliberResourceView>(),
        offset_of!(CaliberResourceView, resource_id),
        offset_of!(CaliberResourceView, generation),
        offset_of!(CaliberResourceView, data),
        offset_of!(CaliberResourceView, len),
        offset_of!(CaliberResourceView, lease),
        size_of::<CaliberTelemetryInfo>(),
        offset_of!(CaliberTelemetryInfo, sequence),
        offset_of!(CaliberTelemetryInfo, schema),
        offset_of!(CaliberTelemetryInfo, reserved),
        offset_of!(CaliberTelemetryInfo, value_count),
        offset_of!(CaliberTelemetryInfo, value_size),
        size_of::<CaliberApiV1>(),
        offset_of!(CaliberApiV1, abi_version),
        offset_of!(CaliberApiV1, struct_size),
        offset_of!(CaliberApiV1, context_create),
        offset_of!(CaliberApiV1, context_destroy),
        offset_of!(CaliberApiV1, context_dispatch),
        offset_of!(CaliberApiV1, context_peek_command),
        offset_of!(CaliberApiV1, context_take_command),
        offset_of!(CaliberApiV1, context_publish_state),
        offset_of!(CaliberApiV1, context_read_latest_state),
        offset_of!(CaliberApiV1, state_publication_release),
        offset_of!(CaliberApiV1, context_map_resource),
        offset_of!(CaliberApiV1, resource_release),
        offset_of!(CaliberApiV1, context_publish_resource),
        offset_of!(CaliberApiV1, context_release_resource),
        offset_of!(CaliberApiV1, context_publish_telemetry),
        offset_of!(CaliberApiV1, context_read_latest_telemetry),
        offset_of!(CaliberApiV1, context_wake_sequence),
        offset_of!(CaliberApiV1, context_wait_wake),
        offset_of!(CaliberApiV1, context_stop_wake_waiters),
    ];
    assert_eq!(
        actual.as_slice(),
        expected.as_slice(),
        "C header and Rust ABI layouts diverged"
    );

    let statuses = [
        CaliberStatus::Ok as i32,
        CaliberStatus::InvalidArgument as i32,
        CaliberStatus::InvalidHandle as i32,
        CaliberStatus::BufferTooSmall as i32,
        CaliberStatus::LimitExceeded as i32,
        CaliberStatus::NotFound as i32,
        CaliberStatus::Stale as i32,
        CaliberStatus::Unavailable as i32,
        CaliberStatus::QueueFull as i32,
        CaliberStatus::UnsupportedVersion as i32,
        CaliberStatus::Internal as i32,
        CaliberStatus::Stopped as i32,
    ];
    assert_eq!(statuses, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
}

#[test]
fn frozen_old_v1_client_loads_current_library() {
    run_client(caliber_old_v1_client_run, "old ABI v1 prefix");
}

#[test]
fn current_v1_client_loads_current_library() {
    run_client(caliber_current_v1_client_run, "current ABI v1");
}

fn run_client(client: unsafe extern "C" fn(*const c_char) -> c_int, label: &str) {
    let path = caliber_library_path();
    assert!(
        path.is_file(),
        "Caliber cdylib missing at {}; run cargo build -p caliber-ffi before ABI compatibility tests",
        path.display()
    );
    let path = CString::new(path.to_string_lossy().as_bytes()).expect("library path has no NUL");
    // SAFETY: each C fixture dynamically loads the cdylib, uses only its frozen
    // or current C declaration, then unloads it before returning.
    let result = unsafe { client(path.as_ptr()) };
    assert_eq!(
        result, 0,
        "{label} client failed at compatibility check {result}"
    );
}

fn caliber_library_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CALIBER_FFI_PATH") {
        return PathBuf::from(path);
    }
    let mut path = std::env::current_exe().expect("test executable path");
    path.pop(); // deps
    path.pop(); // target profile directory
    path.push(if cfg!(target_os = "windows") {
        "caliber_ffi.dll"
    } else if cfg!(target_os = "macos") {
        "libcaliber_ffi.dylib"
    } else {
        "libcaliber_ffi.so"
    });
    path
}
