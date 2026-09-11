# Caliber experimental design

Status: experimental working notes. This document is a hypothesis and test
plan, not a public API or a stability promise.

## Question

Caliber asks whether a native application can keep its domain logic
independent from its presentation technology without introducing a large
runtime, a GUI framework, a renderer, or an expensive request-per-property
boundary.

The smallest useful boundary is:

```text
application logic  ->  application contract  ->  frontend
```

The application defines its own commands, state, events, and resource
formats. The boundary supplies transport, revision, ownership, boundedness,
and wakeup mechanics. A frontend maps application meaning to its own GUI
framework. Caliber does not translate one frontend's widgets into another's.

The experiment is deliberately easy to delete. It does not revive the
Instar or Youth architecture, and it does not define widgets, windows,
layouts, paint commands, audio engines, or application schemas.

## Three planes

The planes are separate concepts even if an early implementation keeps them
in one module or one process.

### Control plane

The control plane carries ordered, low-volume application commands and
semantic events. Examples include `SetSearch`, `SelectSample`,
`PreviewSample`, and `StopPreview`.

Control traffic is bounded and coarse-grained. A command may carry the
revision the frontend observed, but Caliber does not decide whether stale
application input is rejected, transformed, or still valid. The application
does.

The control plane must not become a property RPC surface. Repeated calls such
as `sample_name(0)`, `sample_name(1)`, and `track_gain()` are evidence that the
boundary is too fine-grained.

### State plane

The state plane publishes coherent application-visible state. A publication
has at least:

```text
revision
schema/version
payload
```

The consumer observes either the previous complete publication or the next
complete publication; it never observes a partly replaced value. Rejected
input leaves the last valid publication intact. Full publications are a
reasonable first experiment, but the implementation must not assume that a
full serialized application snapshot is required for every render frame.

The application owns the state schema. Caliber owns only the publication
boundary, revision metadata, limits, and failure behavior.

### Data plane

The data plane is for information where ordinary control/state serialization
is the wrong mechanism. The MVP must distinguish these patterns:

1. **Immutable bulk resources.** A waveform pyramid, image, or large
   analysis result is published once and referenced from state. The reference
   includes an identifier, generation, format, and length. The payload is not
   copied into every state publication or serialized on every repaint.
2. **Latest-value telemetry.** A meter or playhead exposes the newest coherent
   value. Intermediate values may be overwritten. Publication is bounded,
   non-blocking, and allocation-free after setup, with a sequence that lets a
   reader detect change.
3. **Ordered streams.** A scope frame, MIDI event, or capture trace uses a
   bounded SPSC stream when history matters. Full behavior, dropped-data
   counters, and producer/consumer ownership are explicit. A producer never
   blocks on a slow consumer.

These patterns must not collapse into one generic message queue or one
serialization format. In particular, MessagePack may be measured for the
control/state experiment, but it is not a bulk-data mechanism.

## Process model

The initial model is in-process:

```text
frontend executable owns the GUI main thread and event loop
backend is a native module/library
Caliber is the narrow contract between them
```

There is no process supervisor, daemon, network transport, Wasmtime runtime,
sandbox, or async-runtime requirement in the first experiment. The contract
should remain conceptually transportable later, but that possibility is not a
reason to build IPC now.

The frontend owns GUI wake integration. Caliber exposes the fact that work is
available; it does not become a window or event-loop library and does not
require periodic polling while idle.

## Hard realtime exclusion

Caliber is not an audio engine. The realtime audio thread must not depend on
UI responsiveness, GUI event processing, arbitrary callbacks, allocation,
locks, Caliber wakeups, or state serialization.

The intended direction is:

```text
UI -> Caliber -> application/control thread -> realtime bridge -> audio thread
```

Telemetry may originate in realtime processing, but any path from that thread
to Caliber must have a separately audited and benchmarked realtime contract.
No general Caliber function is realtime-safe merely because it is fast in a
microbenchmark. Calling an arbitrary foreign function from an audio callback
is explicitly out of scope.

## Revisions and stale work

Revisions identify coherent state publications. A command may carry
`based_on_revision`; the backend decides the domain semantics of stale input.
Resource identifiers are also generation-qualified so an expired object is
not confused with a later object that reuses its numeric id.

The minimum required behavior is:

```text
valid revision N          -> consumer sees N
malformed attempted N+1   -> consumer still sees N
valid publication N+2     -> consumer atomically sees N+2
```

Caliber must fail closed on malformed, oversized, unknown-version, expired,
or unavailable data. It must not silently reinterpret it as a different
command or resource.

## Boundary size

Cross the boundary by units of application work:

```text
dispatch(command_batch)
read_latest_state()
map(resource_id)
read_latest_telemetry()
read_stream_chunk()
```

The design should remain sensible if a foreign call is much more expensive
than a native Rust call. A state publication can be full in the first proof,
but the data plane must demonstrate that large immutable resources and high
rate telemetry do not become per-frame serialization.

## Wake behavior

The frontend must be able to respond when state, control work, or relevant
data is available without a timer that repeatedly asks whether anything
changed. The first adapter may use the smallest mechanism supported by the
chosen GUI framework. The contract should expose notification intent and
leave the event-loop-specific operation at the edge.

Instrument idle behavior. With no input, state changes, or telemetry
producer, Caliber itself should have no recurring polling work.

## Schema ownership

Caliber does not define Punks' sample schema or a universal dynamic object
model. A Punks-specific fixture may define `SampleSummary`, waveform formats,
and commands, but those are experiment data, not Caliber concepts.

Do not create `CaliberObject`, universal properties, universal methods,
widget translations, or reflective variants. If an experiment appears to need
one, first try a smaller application-specific contract and record the pressure
in `EXPERIMENTS.md`.

## Salvage map

The useful salvage target is behavior and evidence, not old crate structure.

From Instar:

- retain bounded decoder and hostile-input tests;
- retain explicit revision/generation and stale-message tests;
- retain atomic publication and no-poll/suspend tests;
- retain bounded queue and final-consumer backpressure lessons;
- retain headless, forbidden-work, and phase/performance harness patterns;
- retain ownership and failure-semantics documentation.

Do not import Wasmtime, WIT UI, retained trees, widgets, Taffy, windowing,
Vello, paint protocols, or text engines.

From Youth:

- retain command/state boundary examples;
- retain transaction ordering, stale update, bounded input, and malicious
  fixture tests;
- retain deterministic runner and trace ideas;
- retain exact error behavior where it illuminates a generic revision rule.

Do not import the Youth GUI/editor/renderer, SQLite state system, project
format, CLI, capsule packaging, or product-specific protocols.

## Architectural falsifiers

The experiment is not successful merely because one Rust frontend works. It
must be willing to produce a no-go result. Record a no-go if any of these
become true:

- direct frontend/backend integration has lower complexity and no material
  loss of reuse for the tested applications;
- a second frontend requires Caliber-specific widget or GUI abstractions;
- the same application contract cannot be used by substantially different
  frontends without adding presentation concepts to the boundary;
- bulk resources are serialized or copied per repaint;
- latest telemetry requires blocking, unbounded allocation, or history that
  the consumer does not need;
- the boundary requires a polling timer while idle;
- cross-language ownership or shutdown behavior cannot be stated precisely;
- Caliber's incremental binary, memory, latency, or debugging cost dominates
  the flexibility it buys;
- hard realtime safety depends on an unproven property of a generic API.

The direct-integration comparison is mandatory. For each frontend, count
Caliber-only boilerplate, serialization, FFI glue, duplicated models,
debugging cost, latency, binary contribution, and memory contribution. Then
count the actual flexibility gained: backend reuse, independent GUI testing,
frontend replacement, language separation, and deterministic traces. If the
costs clearly dominate, stop.

## What this document does not settle

The experiment does not freeze function names, numeric opcodes, wire layouts,
schema formats, transport choices, or long-term compatibility. Those choices
must follow evidence from the synthetic backend, direct integration, and two
substantially different frontends.
