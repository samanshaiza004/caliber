# Debugging Caliber consumers

Caliber sits between a presentation host and an application-owned contract. A
useful investigation starts by identifying which layer reported the problem:

1. **Caliber status** is a numeric result from one C ABI call. It describes a
   boundary precondition, bounded transport result, lifecycle condition, or
   internal failure.
2. **Application/domain error** is defined by the consuming application. Caliber
   transports its opaque command and state bytes; it does not interpret whether
   a command is valid for the product or whether a requested operation
   succeeded.
3. **Development diagnostic** is output from the `caliber` CLI or from the
   consumer's build, loader, host process, or debugger. It is not a
   `CaliberStatus` returned to the application.

Keep these categories separate in logs and bug reports. In particular,
`CALIBER_STATUS_OK` from `context_dispatch` means Caliber accepted the bytes into
its bounded command queue. It does not mean the application consumed or
successfully applied that command. Likewise, a successful state publication
only confirms that Caliber accepted the opaque publication.

## Public ABI statuses

The names and numeric values below are the current public `CaliberStatus`
values in [`include/caliber.h`](../include/caliber.h). Start with the action in
the last column, then check the referenced call's contract in
[`ABI.md`](ABI.md) or [`LIFECYCLE.md`](LIFECYCLE.md).

| Value | Status | First troubleshooting action |
| ---: | --- | --- |
| 0 | `OK` | No Caliber failure is reported. Check the consumer's application-level result if the requested product operation did not happen. |
| 1 | `INVALID_ARGUMENT` | Check the call's required pointers, lengths, capacities, and configuration fields; confirm every nonzero length has a valid buffer. |
| 2 | `INVALID_HANDLE` | Confirm the context pointer is non-null and is a still-live handle returned by `context_create`; check that shutdown has not destroyed it. |
| 3 | `BUFFER_TOO_SMALL` | Use the call-specific required size or fixed-width metadata to allocate the expected destination, then retry according to that call's contract. |
| 4 | `LIMIT_EXCEEDED` | Compare the payload or requested context limits with the configured and hard limits; reduce the size or choose a valid bound. |
| 5 | `NOT_FOUND` | Check that the resource ID was actually published and has not been retired. |
| 6 | `STALE` | Refresh the resource identity and generation from current application state; do not reuse an old generation. |
| 7 | `UNAVAILABLE` | Check the operation's current availability: for example, whether a queue/state/telemetry sample exists or another wake waiter is active. |
| 8 | `QUEUE_FULL` | Check that the consumer is draining commands, or serialize competing telemetry writers, before retrying. |
| 9 | `UNSUPPORTED_VERSION` | Request ABI version 1 and verify `caliber_get_api(1)` returns a non-null table. Current `caliber_get_api` reports an unsupported version with a null pointer; it does not currently return this status. |
| 10 | `INTERNAL` | Preserve the host's stderr and application logs, record the exact call and inputs, and reproduce under a native debugger with symbols for the loaded Caliber build. |
| 11 | `STOPPED` | Confirm the caller treats this as normal wake-wait shutdown and joins the waiter before destroying the context. |

The status is intentionally coarse and carries no text message. A status does
not by itself prove that the application payload is malformed. Never encode an
application/domain result as a Caliber status; if a boundary operation fails
while carrying a domain request, report the boundary status and application
result independently.

### Call-specific notes

- `BUFFER_TOO_SMALL` from `context_take_command` leaves the command queued and
  writes its required byte count to `out_len`. A retry can then use a buffer of
  that size. Telemetry is fixed-width: allocate the configured telemetry width;
  when a latest sample exists, the output metadata describes that width even
  when the supplied capacity is wrong. See [`ABI.md`](ABI.md).
- `UNAVAILABLE` is operation-dependent. An empty command queue, unpublished
  state, or telemetry with no sample yet can produce it. A second concurrent
  `context_wait_wake` also gets `UNAVAILABLE`; only one waiter is allowed per
  context. See [`LIFECYCLE.md`](LIFECYCLE.md).
- `STOPPED` is specific to the wake-wait lifecycle. It means the wait facility
  was permanently stopped; it does not mean the context or other operations
  have been stopped.
- Caliber can reject a null context with `INVALID_HANDLE`, but it cannot safely
  validate an arbitrary non-null pointer. Invalid, dangling, or concurrently
  destroyed pointers violate the caller contract and are not recoverable status
  cases.

## Application errors and diagnostics

Commands and state are application-defined opaque bytes. The consuming
application should record its own command outcome, validation errors, document
or domain identifiers, and relevant revision. Associate that information with
the Caliber call's status and timestamp without treating it as Caliber-owned
diagnostic data. Avoid logging raw payload bytes unless the application has
established that doing so is safe.

The `caliber` CLI has a separate diagnostic surface. Run commands from the
consumer project root, or provide `--project-root DIR`:

```text
caliber status
caliber doctor
caliber doctor --library PATH
caliber check
caliber check --library PATH
```

`status` reports locked dependency revisions and local managed checkout state;
it does not contact remotes. `doctor` checks the project lock and managed
checkouts, configured host tools, and the configured Caliber library's
loadability, architecture, requested ABI, table extent, and required function
entries. `check` performs those checks and exercises context creation,
command/state/resource calls, and the wake/wait/stop/join lifecycle. A
successful `check` does not launch the consumer's GUI or validate its
application-specific behavior. See [`DEPENDENCIES.md`](DEPENDENCIES.md).

These CLI commands print actionable diagnostics and return a process failure
when a check fails. Such an error is not one of the numeric C ABI statuses.
Review the reported project root, lock/config paths, dependency checkout, tool
found on `PATH`, and actual loaded library path before changing application
code. Use `--library PATH` to verify that the host is inspecting the same
artifact the application loads.

## Host logs and native debugging

Caliber currently has no structured logging or tracing API, log file, or
`RUST_LOG` configuration. The consumer owns host logging. At the application
boundary, record the function name, numeric status, ABI version and library
path, context lifecycle state, buffer lengths/capacities, resource ID and
generation when applicable, and the consumer's application-level result.
Redact application payloads and other sensitive data. Keep the original stderr
and host logs with a failure report.

For `INTERNAL`, reproduce with debug symbols and attach the native debugger to
the process that loads the exact Caliber library under investigation. Break at
the foreign call site and the matching Caliber entry point; inspect the call
arguments, thread, stack, and loaded module path. Use the debugger's Rust panic
break/first-chance exception support if available. On Windows, use Visual
Studio or WinDbg; on macOS, LLDB; on Linux, GDB or LLDB. Ensure symbols match
the exact source revision and binary being loaded.

The FFI entry points contain Rust unwinding with `catch_unwind` where they
return a status; a caught panic is reported as `INTERNAL` rather than unwinding
through C. `INTERNAL` can also reflect an allocation failure or poisoned
internal synchronization. Containment does not recover the failed operation,
expose a panic message through the ABI, or make invalid pointers safe. Rust's
default panic hook may write to the host's stderr before unwinding is caught;
`RUST_BACKTRACE=1` can add a backtrace when that hook runs. With an aborting
panic strategy, the process aborts and `catch_unwind` cannot convert the panic
to a status. Cleanup functions that return `void` cannot report a caught panic
as `INTERNAL`.

## Textual error details

There is no textual error ABI in the current interface. Adding one is deferred
until external consumer testing in issue #8 shows that the numeric statuses,
application-owned diagnostics, and host debugger workflow leave a concrete
diagnostic gap. Do not depend on panic text, Rust implementation details, or
CLI wording as a stable ABI.
