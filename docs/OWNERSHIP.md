# Experimental ownership and threading ledger

This ledger is mandatory for the experiment. It is provisional and must be
updated when an implementation chooses a concrete representation. “Borrowed”
always means the validity interval is explicit; it never means “valid while
the application probably stays alive.”

## Actors

| Actor | Responsibilities | Default thread relationship |
| --- | --- | --- |
| Frontend/UI | GUI framework, event loop, presentation, user input, wake adapter | Owns the UI/main thread |
| Application/control | Domain state, command policy, publication, resource registry | Owns a serialized control context unless an experiment says otherwise |
| Background workers | Indexing, decoding, analysis, preparation of immutable resources | May publish through bounded handoff to the control context |
| Realtime audio | Audio processing and realtime telemetry production | Must not call generic Caliber/GUI/FFI operations |
| Caliber boundary | Bounds, revisions, handles, publication, queue/slot mechanics, errors | Does not become an owner of application meaning |

## Cross-boundary objects

| Object | Allocates | Owns/mutates | Consumer may do | Valid until | Release/shutdown |
| --- | --- | --- | --- | --- | --- |
| Command bytes | Frontend or caller | Caller until dispatch accepts; application interprets after bounded decode | Read during dispatch only | The call's documented slice lifetime | No retained pointer; copy only if the application explicitly queues it |
| State publication | Application/control side | Application creates the next immutable publication; Caliber swaps the visible reference atomically | Read complete publication while leased | Until the matching release; a lease may outlive context destruction | Release exactly once, including after context destruction |
| `ResourceId + generation` | Application registry | Application publishes immutable resource; Caliber tracks lease metadata | Map/read, never mutate | Handle is usable only with a live context and its current generation | Retiring a handle prevents new maps; outstanding mapped views remain valid until released |
| Mapped bulk view | Application/resource registry | Resource owner; consumer is read-only | Read within pointer/length and format bounds | Exactly the lease interval, including across context destruction | Consumer releases exactly once; no shutdown report is provided for outstanding leases |
| Latest telemetry slot | Producer side after setup | One producer publishes a fixed-width `size_t` sample; readers only copy | Read a coherent newest sample and sequence | Until the next coherent read; no retained pointer | Samples overwrite history; context storage is destroyed only after producers/readers quiesce |
| Ordered SPSC stream | Stream creator | One producer writes, one consumer reads | Read surviving ordered entries; cannot block producer | Until consumed or dropped by full-policy | Experimental `caliber-core` facility only; not part of the v0.1 foreign ABI |
| Wake registration | Frontend adapter | Frontend owns callback/event-loop registration; Caliber stores only the minimal token | Trigger notification, not arbitrary reentrant work | Until unregister or backend shutdown | Unregister before destroying the callback target |
| Application context | Application/control owner | Owner mutates all domain state; Caliber mediates calls and bounded channels | Use through documented ABI operations | Until explicit destroy | No operation may race destroy; the raw context pointer is invalid afterward |

## Bulk resource rules

1. A resource must be immutable for the entire lifetime of any mapped view.
2. The pointer/length must refer to stable storage, never to a growable Rust
   container that may move.
3. The view's format, length, id, and generation are checked before access.
4. A consumer cannot retain a view after releasing its lease.
5. The producer cannot reclaim storage while any lease remains.
6. Shutdown makes future maps fail and gives the owner a deterministic way to
   observe outstanding leases; it must not silently free memory still exposed
   to foreign code.

An implementation may choose copying, immutable allocation, or an OS mapping.
No choice is accepted until its cost and lifetime are measured.

## Publication replacement

Publication replacement is atomic from the consumer's perspective. Construct
and validate the next complete publication before making it visible. If
validation fails, keep the old revision and all old resource leases intact.

The minimum trace is:

```text
publish valid N
reject malformed N+1
observe N
publish valid N+2
observe N+2
```

No consumer should observe a payload whose revision does not match its
metadata.

## Thread rules

- UI calls may dispatch bounded control work but must not wait on the audio
  thread.
- Application/control code owns domain mutation and state publication.
- Background workers communicate through bounded, documented handoffs.
- Latest telemetry can be written by a designated producer and read by a
  designated consumer; do not imply arbitrary multi-thread safety.
- The realtime audio thread performs no allocation, lock acquisition, GUI
  callback, state serialization, or generic Caliber wakeup.
- Destruction is a lifecycle operation with an explicit quiescence rule; it
  is not a best-effort drop of foreign pointers.
- Generic ABI calls may allocate or lock and are not realtime-safe. The
  internal telemetry slot's non-blocking write does not make the foreign
  telemetry call realtime-safe because it also advances the wake signal.

## Shutdown checklist

Before a context is destroyed, the experiment must answer:

- Can a frontend still submit a command?
- Can a consumer still read the last state?
- What happens to a held resource view?
- Can a telemetry producer still publish?
- Does a stream report unread or dropped entries?
- Is a registered wake callback unregistered before its target disappears?
- Which thread performs final resource reclamation?

If any answer depends on timing folklore, the ownership contract is not yet
ready for another frontend.
