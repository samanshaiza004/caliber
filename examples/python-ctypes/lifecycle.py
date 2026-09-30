"""Illustrative Python ctypes consumer of Caliber's C ABI v1.

This is an executable example, not a supported Python SDK or maintained
binding. It mirrors the public C records and function table locally so a
consumer can see the ABI calls in one small, dependency-free script. Real
applications should prefer a project-owned/native binding where appropriate.
"""

from __future__ import annotations

import ctypes
import sys
from pathlib import Path


Status = ctypes.c_int32
Context = ctypes.c_void_p
Size = ctypes.c_size_t
U8Pointer = ctypes.POINTER(ctypes.c_uint8)


class ContextConfig(ctypes.Structure):
    _fields_ = [
        ("struct_size", ctypes.c_uint32),
        ("max_command_bytes", Size),
        ("max_publication_bytes", Size),
        ("max_resource_bytes", Size),
        ("max_resources", Size),
        ("telemetry_width", Size),
        ("max_pending_commands", Size),
    ]


class ApiHeader(ctypes.Structure):
    _fields_ = [("abi_version", ctypes.c_uint32), ("struct_size", ctypes.c_uint32)]


class StatePublication(ctypes.Structure):
    _fields_ = [
        ("revision", ctypes.c_uint64),
        ("schema", ctypes.c_uint32),
        ("reserved", ctypes.c_uint32),
        ("data", U8Pointer),
        ("len", Size),
        ("lease", ctypes.c_void_p),
    ]


ContextCreate = ctypes.CFUNCTYPE(Status, ctypes.POINTER(ContextConfig), ctypes.POINTER(Context))
ContextDestroy = ctypes.CFUNCTYPE(None, Context)
ContextDispatch = ctypes.CFUNCTYPE(Status, Context, U8Pointer, Size)
ContextPeekCommand = ctypes.CFUNCTYPE(Status, Context, ctypes.POINTER(Size))
ContextTakeCommand = ctypes.CFUNCTYPE(Status, Context, U8Pointer, Size, ctypes.POINTER(Size))
ContextPublishState = ctypes.CFUNCTYPE(
    Status, Context, ctypes.c_uint32, U8Pointer, Size, ctypes.POINTER(ctypes.c_uint64)
)
ContextReadState = ctypes.CFUNCTYPE(Status, Context, ctypes.POINTER(StatePublication))
StateRelease = ctypes.CFUNCTYPE(None, ctypes.POINTER(StatePublication))
ContextWakeSequence = ctypes.CFUNCTYPE(Status, Context, ctypes.POINTER(ctypes.c_uint64))
ContextWaitWake = ctypes.CFUNCTYPE(
    Status, Context, ctypes.c_uint64, ctypes.POINTER(ctypes.c_uint64)
)
ContextStopWakeWaiters = ctypes.CFUNCTYPE(Status, Context)


class ApiV1(ctypes.Structure):
    _fields_ = [
        ("abi_version", ctypes.c_uint32),
        ("struct_size", ctypes.c_uint32),
        ("context_create", ContextCreate),
        ("context_destroy", ContextDestroy),
        ("context_dispatch", ContextDispatch),
        ("context_peek_command", ContextPeekCommand),
        ("context_take_command", ContextTakeCommand),
        ("context_publish_state", ContextPublishState),
        ("context_read_latest_state", ContextReadState),
        ("state_publication_release", StateRelease),
        # Resource and telemetry slots remain here to preserve the public
        # function-table offsets, though this small sample does not call them.
        ("context_map_resource", ctypes.c_void_p),
        ("resource_release", ctypes.c_void_p),
        ("context_publish_resource", ctypes.c_void_p),
        ("context_release_resource", ctypes.c_void_p),
        # The current table also contains telemetry entries, which this short
        # example does not call. Keep their slots in place before the wake tail.
        ("context_publish_telemetry", ctypes.c_void_p),
        ("context_read_latest_telemetry", ctypes.c_void_p),
        ("context_wake_sequence", ContextWakeSequence),
        ("context_wait_wake", ContextWaitWake),
        ("context_stop_wake_waiters", ContextStopWakeWaiters),
    ]


OK = 0
STOPPED = 11
ABI_VERSION_1 = 1


def require_status(actual: int, expected: int, operation: str) -> None:
    if actual != expected:
        raise RuntimeError(f"{operation} returned Caliber status {actual}, expected {expected}")


def byte_array(value: bytes):
    return (ctypes.c_uint8 * len(value)).from_buffer_copy(value)


def main(library_path: Path) -> None:
    # Keep the CDLL object alive until after context destruction so its function
    # table and code remain mapped for the full context lifetime.
    library = ctypes.CDLL(str(library_path))
    get_api = library.caliber_get_api
    get_api.argtypes = [ctypes.c_uint32]
    get_api.restype = ctypes.POINTER(ApiHeader)
    api_header_pointer = get_api(ABI_VERSION_1)
    if not api_header_pointer:
        raise RuntimeError("library does not provide Caliber ABI v1")
    api_header = api_header_pointer.contents
    if api_header.abi_version != ABI_VERSION_1:
        raise RuntimeError("library returned an incompatible Caliber ABI version")
    if api_header.struct_size < ctypes.sizeof(ApiV1):
        raise RuntimeError("Caliber ABI v1 table is shorter than this example requires")
    # Only cast to/read the full layout after the header confirms its size.
    api = ctypes.cast(api_header_pointer, ctypes.POINTER(ApiV1)).contents
    required_functions = (
        "context_create",
        "context_destroy",
        "context_dispatch",
        "context_peek_command",
        "context_take_command",
        "context_publish_state",
        "context_read_latest_state",
        "state_publication_release",
        "context_wake_sequence",
        "context_wait_wake",
        "context_stop_wake_waiters",
    )
    missing = [name for name in required_functions if not getattr(api, name)]
    if missing:
        raise RuntimeError(f"Caliber ABI v1 table has missing entries: {', '.join(missing)}")

    context = Context()
    state_view = StatePublication()
    try:
        config = ContextConfig()
        config.struct_size = ctypes.sizeof(ContextConfig)
        require_status(api.context_create(ctypes.byref(config), ctypes.byref(context)), OK, "create")

        # Caliber transports opaque command bytes. The application interprets
        # them and is responsible for deciding what state to publish.
        command = b'{"op":"open","path":"notes.md"}'
        command_bytes = byte_array(command)
        require_status(
            api.context_dispatch(context, command_bytes, len(command)), OK, "dispatch"
        )
        command_size = Size()
        require_status(api.context_peek_command(context, ctypes.byref(command_size)), OK, "peek")
        received = (ctypes.c_uint8 * command_size.value)()
        command_taken = Size()
        require_status(
            api.context_take_command(
                context, received, command_size.value, ctypes.byref(command_taken)
            ),
            OK,
            "take",
        )
        if bytes(received[: command_taken.value]) != command:
            raise RuntimeError("application received different command bytes")

        # This illustrative application has consumed the command and publishes
        # its latest state. A pre-publication sequence prevents a lost wake.
        observed_sequence = ctypes.c_uint64()
        require_status(
            api.context_wake_sequence(context, ctypes.byref(observed_sequence)), OK, "observe wake"
        )
        state = b'{"document":"notes.md","open":true}'
        state_bytes = byte_array(state)
        state_revision = ctypes.c_uint64()
        require_status(
            api.context_publish_state(
                context, 1, state_bytes, len(state), ctypes.byref(state_revision)
            ),
            OK,
            "publish state",
        )
        changed_sequence = ctypes.c_uint64()
        require_status(
            api.context_wait_wake(
                context, observed_sequence.value, ctypes.byref(changed_sequence)
            ),
            OK,
            "wait for state publication",
        )
        if changed_sequence.value == observed_sequence.value:
            raise RuntimeError("wake wait returned without observing a change")

        require_status(
            api.context_read_latest_state(context, ctypes.byref(state_view)), OK, "read state"
        )
        if (
            state_view.revision != state_revision.value
            or state_view.schema != 1
            or ctypes.string_at(state_view.data, state_view.len) != state
        ):
            raise RuntimeError("state lease does not match the latest publication")
        api.state_publication_release(ctypes.byref(state_view))

        require_status(api.context_stop_wake_waiters(context), OK, "stop wake waiters")
        stopped_sequence = ctypes.c_uint64()
        require_status(
            api.context_wait_wake(context, changed_sequence.value, ctypes.byref(stopped_sequence)),
            STOPPED,
            "wait after stop",
        )
        print(f"Illustrative Python ctypes consumer passed (state revision {state_revision.value}).")
    finally:
        # Every lease must be released before its context is destroyed.
        if state_view.lease:
            api.state_publication_release(ctypes.byref(state_view))
        if context.value:
            api.context_stop_wake_waiters(context)
            api.context_destroy(context)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} PATH_TO_CALIBER_SHARED_LIBRARY")
    main(Path(sys.argv[1]).resolve())
