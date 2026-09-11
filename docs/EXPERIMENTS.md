# Caliber experiment plan

Status: provisional. This file records experiments and stop conditions, not a
roadmap to a supported framework.

## Order of work

### 1. Synthetic backend and trace

Start with a tiny backend containing a sample library, search query, selected
sample, preview flag, fake immutable waveform, and fake meter. It must not
depend on Punks, a GUI, an audio device, Wasmtime, or a network transport.

Exercise the three planes:

- control: search, select, preview, stop;
- state: atomic revisioned summaries and resource references;
- data: one immutable waveform, latest-value meter, and an optional bounded
  ordered stream.

Replay a deterministic trace without a GUI. A malformed next publication must
leave the previous valid revision visible. Stale command revisions must reach
the backend's policy rather than being silently accepted or silently rejected
by Caliber.

The checked-in dependency-free harness under `experiments/synthetic/` is a
model and test fixture, not production core behavior.

The Rust companion under `experiments/synthetic-rust/` is the first native
falsification harness. It depends only on the private `caliber-core` crate,
keeps all sample/search/preview/favorite schemas local to the experiment, and
exercises the actual core mechanisms without Punks or a GUI:

- ordered semantic commands with an application-owned stale-revision policy;
- atomic revisioned state publication and rejected-publication preservation;
- a bounded Postcard state fixture whose favorite/resource fields have fixed
  cardinality;
- one immutable waveform resource reference, including lease/release behavior;
- latest-value meter publication with overwritten intermediate values;
- deterministic command-trace replay.

Run it with:

```text
cargo test --manifest-path experiments/synthetic-rust/Cargo.toml
cargo run --manifest-path experiments/synthetic-rust/Cargo.toml
```

This is intentionally a falsification harness rather than a proposed SDK. Its
direct-integration comparison is the next required step: measure what the
boundary costs before adding a frontend or generalizing any schema.

### 2. Ownership and failure probes

Probe null/oversized/malformed input, unknown versions, unknown or expired
resource generations, resource release, backend shutdown, telemetry reset,
stream saturation, and wake unregister. Record exact errors and preservation
of the last valid state.

Add hostile input fixtures from Instar and Youth only where they test these
generic boundaries. Do not port product-specific schemas or runtime code.

### 3. Direct-integration kill test

For each frontend candidate, build or estimate the equivalent direct
application-to-GUI integration. Record:

- boundary-only boilerplate;
- duplicated domain models;
- serialization and FFI glue;
- debugging and shutdown complexity;
- command-to-application and state-to-UI latency;
- incremental binary and memory cost;
- idle CPU and polling behavior.

Then record what Caliber bought:

- unchanged backend reuse;
- backend tests without a GUI;
- frontend replacement;
- language separation;
- deterministic traces;
- reuse of immutable resources and latest-value telemetry.

If the direct implementation is plainly simpler with no material loss of
flexibility, record **NO-GO** and stop. Do not add features to rescue the
architecture after the kill test fails.

### 4. First frontend

Use one native frontend to expose mistakes in the contract. It must search,
select, start/stop preview, display metadata and indexing progress, draw a
waveform from one bulk resource, and show a moving playhead and live meter.

The waveform must not be serialized each repaint. Drive a synthetic producer
faster than the UI reads the meter and measure overwrites, coherent reads,
allocations, and contention.

### 5. Second frontend

Use a substantially different GUI/runtime and, preferably, a foreign language
such as Go. It must consume the same backend contract and implement the same
logical features. If a framework is blocked by a concrete platform problem,
record the reason and choose another genuinely different frontend. Rewriting
the backend contract to suit one frontend is a failed experiment.

The two frontends need not share UI code. That duplication is intentional:
Caliber separates application meaning from presentation; it does not promise
write-once UI.

The current foreign proof is a small Go cgo smoke frontend. It intentionally
has no GUI and only verifies that a foreign caller can use the versioned table,
bounded command FIFO, state/resource leases, telemetry slot, and wake signal.
It is useful ABI evidence, but it does not count as the required second GUI
consumer; that remains an explicit stop/continue decision in
`EXPERIMENT-RESULTS.md`.

### 6. Performance and idle evidence

Measure, without marketing targets:

- command dispatch to backend receipt;
- state publication to frontend observation;
- latest telemetry publish to read;
- immutable resource map/access;
- median, p95, and p99 where meaningful;
- allocation counts, steady-state RSS, startup allocations, idle CPU, and
  release binary contribution.

Compare direct integration and Caliber incrementally. Do not compare unrelated
GUI framework runtimes and attribute that difference to Caliber.

An idle test has no user input, state changes, or telemetry producer. Caliber
must not run a polling timer merely to discover that nothing changed.

### 7. Punks, only after the synthetic proof

If the mechanism survives the synthetic and direct-integration tests, add a
small Punks facade. The facade, not the audio engine, participates in the
boundary. Keep audio processing and realtime bridges outside generic Caliber
calls. The Punks facade may expose search, selection, preview, favorites,
index status, waveform references, and meter telemetry as application-owned
schemas.

## Falsification matrix

| Claim under test | Evidence that would falsify it |
| --- | --- |
| Three planes reduce needless work | Bulk data is copied/serialized per frame, or telemetry history is forced through state/control |
| Revisioned state is coherent | Consumer observes mixed metadata/payload or rejected N+1 replaces valid N |
| Resource handles are safe | A view outlives a generation, points into moving storage, or shutdown frees leased memory |
| Latest-value telemetry fits | Producer blocks/allocates, reads are torn, or the UI needs historical values to be correct |
| SPSC stream is sufficient | MVP requires multiple producers/consumers or an unbounded queue to preserve semantics |
| Native boundary is small | A frontend needs widget, layout, window, renderer, or universal-object concepts in the contract |
| Cross-language boundary is useful | Foreign frontend spends more effort on glue/debugging than the reused backend saves |
| Idle is real | Caliber polls or wakes repeatedly without application work |
| Realtime exclusion is credible | Generic Caliber calls or wakeups appear on the audio thread |
| Caliber buys flexibility | Direct integration is simpler and the second frontend adds no meaningful reuse |

## Required failure semantics

The experiment must test malformed commands, oversized packets, unknown
schema/version, stale revision, invalid/expired resources, held resources at
shutdown, telemetry reset, stream saturation, frontend disconnect, backend
failure, and invalid ABI pointers where the contract allows detection.

Each failure must fail closed, preserve prior valid state when appropriate,
and produce an observable error. Silent reinterpretation is a failure.

## Exit decisions

Record one of these outcomes with evidence:

- **NO-GO:** direct integration wins or the boundary creates more complexity
  than flexibility;
- **continue privately:** the mechanism is promising but API/ownership or
  frontend evidence is incomplete;
- **narrow extraction candidate:** two independent consumers and a stable,
  framework-neutral differentiator exist.

Do not publish or promise compatibility merely because the synthetic backend
passes. Passing tests establish a mechanism worth testing further, not a
library worth maintaining.
