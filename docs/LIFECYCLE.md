# Caliber ABI v1 lifecycle

This guide describes the public C boundary in [`include/caliber.h`](../include/caliber.h).
The detailed ownership ledger is in [`OWNERSHIP.md`](OWNERSHIP.md).

## Context and concurrent calls

The caller owns each context returned by `context_create` and must destroy it
exactly once. Normal context operations use internal synchronization and may
overlap, subject to these limits:

| Operation | Concurrency rule |
| --- | --- |
| Commands | Dispatch, peek, and take are synchronized. Accepted packets are copied; the bounded queue can return `QUEUE_FULL`. |
| State | Publishers are serialized; readers receive immutable snapshots and may overlap publication. |
| Resources | Publish, map, and retire operations are synchronized. A mapped view owns a lease independent of the registry. |
| Telemetry | One producer and multiple readers are supported. Samples have the configured fixed width; a competing writer can return `QUEUE_FULL`. |
| Wake wait | At most one waiter may be active for a context. A second concurrent waiter returns `UNAVAILABLE`. Other context operations can run while the waiter blocks. |
| Context destroy | Must not overlap any context operation, producer, reader, or waiter. The caller must establish quiescence. |

Calls may allocate or lock and are not realtime-safe. In particular, the
foreign telemetry function also advances the wake signal, so the internal
slot's allocation-free writer does not make the ABI call suitable for an audio
callback.

## Leases and generations

A successful state read or resource map creates one lease. Keep its pointer
valid only until the matching release function is called, and release each
successful lease exactly once. Releasing a resource ID retires the registry's
owner reference and prevents future maps through that ID/generation pair; a
view that is already mapped remains valid. A reused ID receives a new
generation, so an old handle cannot select the new resource.

State and resource lease storage is independently owned. Existing leases stay
valid after context destruction and can still be released then. For a clean
shutdown, release them before destroying the context. The ABI does not report
outstanding leases during destruction, so the caller must track and release
them.

## Wake and stop behavior

`context_wake_sequence` reads a change sequence. `context_wait_wake` blocks
until that sequence differs from the observed value or the wait facility has
been stopped. A successful return carries the latest sequence; multiple
publications may coalesce, so it is not an event count. Spurious condition
variable notifications do not satisfy the sequence predicate.

`context_stop_wake_waiters` is permanent and idempotent. It wakes an active
waiter, which returns `STOPPED` if it observes the stop before returning a
sequence change. Later wait calls also return `STOPPED`. If another waiter is
already active, a concurrent wait returns `UNAVAILABLE`. Stop only ends the
wake-wait facility: it does not stop producers or reject other context calls.
The application must stop and quiesce its own producers before destruction.

## Shutdown order

Use this order:

1. Stop application workers and prevent new context calls through
   application-owned coordination.
2. Call `context_stop_wake_waiters`.
3. Join the frontend-owned waiter thread.
4. Release every state and resource lease.
5. Call `context_destroy` exactly once.

The context pointer is invalid after step 5. Calling any context function with
it, or destroying it a second time, violates the caller contract and cannot be
reported as a `CaliberStatus`.

## Failure results relevant to lifecycle

- `context_wait_wake` returns `STOPPED` after the wait facility stops and
  `UNAVAILABLE` when another waiter is active.
- Mapping an unknown resource ID returns `NOT_FOUND`; mapping a retired or
  mismatched generation returns `STALE`.
- A telemetry writer that overlaps another writer can return `QUEUE_FULL`;
  readers copy a complete sample or receive a status without a partial copy.
- Invalid non-null pointers and calls after context destruction are caller
  violations, not recoverable status results.

The repository's ABI tests cover a held state lease after context destruction,
a retired resource lease after destruction, stale generations, competing
waiters, idempotent stop, stop-wake shutdown, telemetry metadata, and old/current
v1 clients. Project consumers additionally run their native start/stop smokes;
the headless Caliber `check` command does not replace those GUI-host checks.
