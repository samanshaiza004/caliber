# Caliber C ABI

Caliber is pre-v0.1. Its first v0.1 release is intended to promise the C ABI v1
as the stable, append-only foreign-language boundary. That promise starts with
the release; pre-v0.1 candidates may still evolve. The canonical declaration
is [`include/caliber.h`](../include/caliber.h). Consumers include that file
from their exact Caliber source dependency; they do not maintain local copies
of the Caliber records or function table.

The C header is the ABI declaration source of truth. The Rust implementation
uses `#[repr(C)]` records and is checked against the C compiler's size and
offset results in `crates/caliber-abi-tests`. CI also builds and runs both a
frozen pre-wake v1 client and a client using the current header against the
current shared library. This avoids an IDL or a language-specific binding
generator while mechanically detecting drift.

## Compatibility rules

- The v0.1.x compatibility promise covers only the public C ABI described by
  this header and this document: `caliber_get_api(1)`, public record layouts,
  status values, function-table prefix rules, and ownership/lifecycle
  semantics. It does not cover Rust APIs, crate versions, CLI behavior, lock
  schema, or application-owned protocols.
- `CaliberStatus` is a signed `int32_t`; status numbers and the meaning of
  existing fields are permanent within v1.
- Never reorder, remove, resize, or repurpose an existing v1 field or function
  table entry.
- New `CaliberApiV1` entries are appended. A caller checks `abi_version` and
  checks `struct_size` through the end of an entry before reading or calling it.
- `CaliberContextConfig.struct_size` describes the caller's available prefix.
  The library reads only complete fields within that prefix and defaults fields
  that are absent.
- `size_t` is used for byte lengths and native word-sized telemetry. A caller
  must match the library's process architecture and pointer width.
- Any incompatible representation or semantic change requires a new ABI
  version and a new entry point. Do not silently reinterpret v1.
- Before the first v0.1 release, the candidate may be revised. After that
  release, an incompatible change requires a new table type and
  `caliber_get_api(2)`; v1 remains available to supported v0.1.x consumers.

The compatibility fixture in
`crates/caliber-abi-tests/tests/fixtures/v1-prefix/caliber_v1_prefix.h` represents the
published v1 prefix before the two blocking wake functions were appended. It
is intentionally frozen; changing it would weaken the regression.

## Data-plane v0.1 decisions

- The foreign ABI exposes immutable resources and latest-value telemetry. It
  does not expose an ordered-stream API in v0.1. The SPSC stream in
  `caliber-core` remains an internal Rust facility, not a foreign compatibility
  promise. A future stream API must be justified by a real consumer and can be
  added to v1 only as a compatible append-only table extension; otherwise it
  requires a new ABI version.
- A telemetry sample is one fixed-width array of exactly
  `CaliberContextConfig.telemetry_width` native `size_t` values. The application
  defines each value's meaning. `value_size` reports `sizeof(size_t)`;
  `schema` and `reserved` are zero in ABI v1. A successful publication replaces
  the previous complete sample, increments its sequence, and retains no
  history. Reads copy a coherent sample into caller-owned storage.
- This is an in-process native ABI, not a cross-architecture wire format.
  Callers must match the library's pointer width. No generic Caliber operation
  is promised to be realtime-safe; ABI calls may allocate or lock.

For concurrency, lease lifetime, wake shutdown, and the required destroy order,
see [`LIFECYCLE.md`](LIFECYCLE.md).

## Ownership and call behavior

- Caliber owns each context returned by `context_create`; the caller must call
  `context_destroy` exactly once after all operations and waiter threads have
  stopped. Destruction must not race another call.
- `context_dispatch` copies command bytes before returning. Its input can be
  released after the call.
- A successful state read returns an immutable lease. Keep its data pointer
  valid only while that lease is held, then call
  `state_publication_release` exactly once. The lease can outlive the context.
- A successful resource map similarly returns a lease released with
  `resource_release`. This is distinct from `context_release_resource`, which
  retires the id/generation pair from the context.
- Telemetry is copied into caller-owned storage; no returned telemetry pointer
  or lease exists.
- No Rust panic crosses the C boundary. Invalid pointers remain a caller
  contract violation. A non-zero length requires a non-null pointer.
- Calls may allocate or take locks. They are not suitable for hard realtime
  callbacks.

## Wake behavior

`context_wake_sequence` provides a change sequence. `context_wait_wake` blocks
until it differs from the observed value or waiters are stopped. Only one
waiter may be active per context. `context_stop_wake_waiters` is permanent and
idempotent; it wakes the waiter, which returns `CALIBER_STATUS_STOPPED` (11).
The frontend owns the waiter thread and must stop and join it before destroying
the context. Wake notifications may coalesce and do not represent a count of
every intermediate update.

## Verification

From a clean Caliber checkout, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
```

The ABI compatibility tests dynamically load the built `caliber_ffi` library.
They fail if it is missing. They compare all public record/table offsets and
sizes between C and Rust, check status values, then exercise the frozen old
client and the current client against the library. CI runs these checks on
Windows, macOS, and Linux.
