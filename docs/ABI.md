# Experimental ABI notes

Status: provisional design constraints for experiments. This is not a
published ABI, compatibility guarantee, or final function-table specification.

## Purpose

The first foreign-language seam should be small, C-compatible, and easy to
audit. The canonical implementation may be Rust, but foreign callers must
not depend on Rust layout, ownership, or panic behavior.

The ABI transports application work; it does not expose a universal object
model or GUI vocabulary.

## Candidate shape

Investigate one versioned entry point that returns a function table, rather
than exporting a large set of unrelated symbols:

```text
caliber_get_api(requested_version) -> api_table or explicit error
```

This is a candidate for the experiment, not a commitment. The function table
must have an explicit byte size and version so a caller can reject an
incompatible table without reading beyond what it received.

The smallest useful operations are likely to cover:

- open/create and destroy an application context;
- dispatch a bounded command batch;
- read or acknowledge a complete state publication;
- map and release an immutable resource;
- read latest telemetry;
- read a bounded ordered-stream chunk;
- register or trigger frontend wake integration.

Exact signatures should be written only after ownership tests. Do not add
helpers for individual application fields or widgets.

## Representations

Use representations that a C, Go, Odin, or similar caller can state clearly:

- fixed-width integer identifiers;
- explicit enum/tag values with documented integer widths;
- pointer-plus-length byte or element slices;
- caller-provided output buffers where bounded output is practical;
- opaque handles whose owner and release operation are explicit;
- result codes or tagged result records with an explicit error domain.

Do not expose Rust `String`, `Vec`, `HashMap`, trait objects, `Arc`, Rust
references, or Rust enums without an explicit ABI representation. Do not let
a panic cross the boundary.

Every byte-oriented input needs a maximum size before decoding. Rejection
must leave the last valid state/publication untouched.

## Thread and call contracts

Each operation needs an explicit answer to:

| Question | Required answer before implementation is accepted |
| --- | --- |
| Which thread may call it? | UI/main, application/control, background, or a named SPSC side |
| May calls overlap? | Single-owner, serialized, or a documented concurrent operation |
| Who may block? | Prefer no blocking in frontend-facing calls; prove any exception |
| May it allocate? | State whether setup-only, normal control, or forbidden |
| Is it realtime-safe? | Default is no; only separately audited primitives may say yes |
| What does shutdown do? | Handles become explicitly invalid; no use-after-shutdown |
| What does failure preserve? | Prior valid state and resources remain valid unless stated otherwise |

The ABI must not force foreign frontends to infer thread affinity from
implementation details.

## State publication candidate

A publication should be consumed as one complete object. An implementation
may use a borrowed pointer/length view while the publication is acknowledged,
or an owned buffer that is released explicitly. The choice belongs in the
ownership ledger and must be tested under replacement and shutdown.

At minimum the metadata needs:

```text
revision
schema/version
payload length
payload view or owned handle
```

The payload encoding is application-owned. A first experiment may measure one
mature bounded encoding such as MessagePack, but Caliber must not imply that
encoding is suitable for bulk resources or realtime telemetry.

## Resource handles

An immutable resource handle should include an id and generation, and expose
format and length metadata without requiring the consumer to parse the bulk
payload first. A mapped view is valid only while the stated resource lease is
held. The API must distinguish:

- unknown id;
- expired generation;
- wrong format or bounds;
- backend shutdown;
- consumer release.

Never hand foreign code a pointer into a container that may move or mutate.
The implementation may use an immutable allocation, operating-system mapping,
or another mechanism, but the choice must be visible in `OWNERSHIP.md`.

## Latest-value telemetry candidate

The initial experiment should prefer a fixed-size preallocated slot or a
well-understood sequence-lock/double-buffer pattern. The reader must obtain a
coherent value and a publication sequence; it may observe a newer value than
the one it expected and may miss intermediate updates.

The producer must not allocate, block, call into GUI code, or depend on a
consumer being scheduled. This is a data-plane primitive, not a general FFI
callback facility.

## Ordered stream candidate

Use a bounded SPSC ring-buffer style design or an established implementation
after its contract has been checked. The MVP needs one producer and one
consumer, fixed capacity, non-blocking producer behavior, explicit full/drop
semantics, and measurable counters. Do not turn this into an unbounded queue
or a multi-producer abstraction without evidence.

## Wake integration

The core may report that work became available; the frontend adapter owns the
GUI event-loop operation. Wake registration must state which thread performs
the callback and whether coalescing is allowed. A wake is a notification, not
a promise that a particular payload remains available forever.

The adapter must not require a timer just to observe changes. An idle test
should show no recurring Caliber polling work.

## ABI rejection and fuzzing

Reject null pointers where a non-null slice is required, overflowed lengths,
unknown versions, oversized packets, invalid handles, expired generations,
and malformed payloads. Return an explicit error; do not panic or reinterpret
the bytes.

Fuzz every decoder and handle path. The important properties are:

- no panic or undefined behavior;
- bounded allocation and work;
- prior valid state survives rejected publication;
- a released or expired resource is never dereferenced;
- shutdown invalidates outstanding operations deterministically.

## Stability restraint

The ABI is intentionally not promised stable. Do not publish a crate, assign
semver policy, or write frontend bindings until a synthetic backend and at
least two materially different frontends demonstrate that the boundary buys
more than it costs. A future ABI may change or be deleted if direct
integration wins.
