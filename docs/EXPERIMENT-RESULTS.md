# Caliber experiment results

Current status: **experimental, being independently dogfooded, no stable API
yet**. The current verdict is recorded at the end of this document.

The extraction is now an independent repository with the original Caliber
implementation history preserved as its root commit. The first real pressure
test is the Scratchpad semantic boundary documented in
[`SCRATCHPAD-DOGFOOD.md`](SCRATCHPAD-DOGFOOD.md).

## Current evidence

- `caliber-core` provides bounded control, revisioned state,
  generation-checked immutable resources, latest-value telemetry, SPSC
  overflow behavior, and a coalescing wake signal.
- The Rust synthetic backend exercises all three planes with application-owned
  schemas, a waveform resource, stale-command policy, malformed-state
  preservation, and deterministic trace replay.
- The provisional C table has explicit pointer/length checks, bounded queues,
  lease cleanup, generation errors, short-config handling, panic containment,
  and Rust round-trip tests.
- The Go cgo smoke frontend consumes the same table and exercises command
  dispatch/peek/take, state publication/read, immutable resource mapping and
  release, latest telemetry, wake observation, and cleanup.
- Linux, macOS, and Windows CI now cover formatting, clippy, Rust tests, and
  workspace buildability. Unix CI also runs the synthetic and Go/cgo proofs.
- Scratchpad's current Shirei UI uses a small typed semantic contract for
  open/select/save/close while leaving its scalable editor and GUI mechanics
  intact. The proof is committed on the `caliber-dogfood` branch at
  [`8bfa8a6`](https://github.com/samanshaiza004/scratchpad/commit/8bfa8a6),
  and Scratchpad's application package remains Shirei-free.

## What the dogfood changed

The real application made the boundary smaller and more concrete. The useful
contract is document lifecycle and shell state, not editor mechanics. File
identity, open order, active selection, dirty/conflict status, save policy, and
close policy are application-authoritative. Text buffers, caret/selection,
viewport, shaping, folds, projections, and paint remain local to the existing
Scratchpad/Shirei path.

That is enough to keep the current UI replaceable in principle: a future shell
can implement against the semantic contract without importing Shirei. It is not
enough evidence to justify forcing that contract through the current Rust C ABI
yet. The direct Go adapter has no serialization, no foreign handles, no new
thread, and no shutdown protocol. A future foreign client would need to earn
those costs with a second real consumer.

## Cost record

The Scratchpad patch added 129 lines for the contract, 124 lines of tests, and
58 added lines of routing/revision changes. It copies only a bounded
document-summary slice when a snapshot is requested; it does not copy editor
bytes per frame.
The local dispatch path adds an interface call and switch but no encoding or
allocation in the tested lifecycle commands. It adds no goroutine and does not
change shutdown or ownership behavior.

The extracted `caliber-core` and `caliber-ffi` crates contain 2,775 raw Rust
source lines including tests and documentation comments. On the development
Apple M1 host, a release build produced a 419 KiB dynamic library and a 17 MiB
static library. These are standalone reference artifacts, not incremental
Scratchpad sizes, because the first dogfood intentionally did not link the FFI
library.

On the development Apple M1 host, three benchmark runs measured approximately
9.5–10.3 ns/op and 0 B/op for semantic select dispatch, versus 9.6–13.1 ns/op
and 0 B/op for direct `Activate`. A one-document snapshot measured 48–56 ns/op,
64 B/op, and one allocation. These are local Go contract measurements, not
Rust/FFI latency targets.

Incremental RSS and binary size were not reported as Caliber numbers because
the first dogfood slice intentionally does not link `caliber-ffi`. Reporting a
native-boundary number here would confuse a direct Go adapter with a Rust/FFI
integration. The standalone core's copy and lease
behavior is stated in the design/ownership docs and covered by tests; a
cross-language benchmark remains a required future experiment, not a claimed
result.

The direct Shirei comparison therefore currently wins on simplicity. Caliber's
present gain is architectural evidence: an explicit, headless-testable,
Shirei-free application contract and preserved backend/editor ownership. The
second frontend and measured cross-language reuse are still absent.

## Historical result from the Instar-only experiment

The following is retained as historical evidence. It was correct for the
earlier, Instar-contained experiment and is not the current repository status.

### Evidence completed then

- `caliber-core` provided bounded control, revisioned state, generation-checked
  immutable resources, latest-value telemetry, SPSC overflow behavior, and a
  coalescing wake signal.
- The Rust synthetic backend exercised all three planes with application-owned
  schemas, a single waveform resource, stale-command policy,
  malformed-publication preservation, and deterministic trace replay.
- The provisional versioned C table had explicit pointer/length checks,
  bounded queues, lease cleanup, generation errors, short-config handling,
  panic containment, and Rust round-trip tests.
- The Go cgo smoke frontend consumed the same table and exercised command
  dispatch/peek/take, state publication/read, immutable resource mapping and
  release, latest telemetry, wake observation, and cleanup.
- The idle queue test demonstrated that the core did not require a polling
  timer merely to wait for control work.

### Evidence deliberately missing then

- There was no second real GUI frontend. No Fenestra or Shirei checkout was
  available in the original experiment, so no substitute GUI framework was
  invented.
- There was no Punks facade. The sibling Punks checkout was independently dirty
  and was not modified; integrating it before the synthetic proof would have
  coupled the experiment to a large presentation/runtime dependency.
- No benchmark numbers were advertised. The direct-integration kill test still
  needed measured boilerplate, latency, allocations, RSS, and debugging/
  shutdown cost for a real application.
- The C table was not stable and had no generated public header or bindings.
  It was a seam to falsify, not a package to publish.

### Historical evaluation

The language split was technically possible: the Go smoke caller consumed the
same C table as the Rust tests. That was not evidence of a useful UI split,
because it had no GUI and there was no second real frontend. GUI-framework
neutrality, waveform repaint behavior, and application-level debugging cost
therefore remained unproven.

For the only real consumer at that time, the direct Rust synthetic application
was plainly smaller than carrying a 2,600-line core/ABI boundary plus lease
and pointer contracts. The boundary bought no demonstrated reuse until a
second independent frontend existed. The required Punks facade, Fenestra
frontend, second foreign GUI, latency/RSS/binary measurements, and foreign
stream/wake adapter were intentionally not invented to rescue that result.

Historical verdict: **NO-GO** for publishing or splitting the mechanisms at
that point. The finding remains valid as historical evidence, but extraction
and Scratchpad dogfood now test the narrower application-lifecycle hypothesis.

## GPUI second-frontend dogfood

Scratchpad's `gpui-dogfood` branch now supplies the first materially different
frontend attempt: Rust/GPUI shell → one Go c-shared backend → one linked
Caliber `cdylib` → the unchanged Shirei-free Go application contract. The Rust
package depends on `gpui-kit = "=0.6.1"` only; it does not depend on
`caliber-ffi`.

The Gate 1 interface remains intentionally small: bounded shell state and one
requested directory listing, plus open/select/save/close commands. Rust calls
the Caliber API table returned by the Go backend directly for dispatch, state
read/release, and wake observation. A single GPUI background scheduler owns the
session and serializes filesystem work; the foreground only submits semantic
commands. Numeric request IDs correlate outcomes, application revisions are
kept separate from Caliber transport revisions, and invalid UTF-8 paths are
rejected explicitly.

The acceptance suite passes the root Scratchpad tests without Caliber, nested
backend tests with `GOEXPERIMENT=cgocheck2`, and Rust format/tests/clippy/build.
Lifecycle tests cover double start/stop, call after stop, lease-protected
shutdown, malformed/oversized packets, and bounded listings. A macOS loader
inspection of the built backend shows exactly one Caliber dynamic dependency;
the Rust executable has no Caliber dependency. The local managed environment
cannot run the AppKit executable to completion, so native launch smoke remains
a CI/desktop-worker check rather than a claimed local pass.

The measured debug artifact set is three files: approximately 92 MiB Rust
executable, 24 MiB Go c-shared backend, and 0.8 MiB Caliber cdylib. These are
engineering-cost signals, not release sizes. Cold start and settled idle RSS
still need a native sampler. The concrete new costs are the Go runtime
baseline, JSON encode/decode and bounded state copies, three-artifact
packaging, dynamic-loader setup, and explicit lease/lifetime diagnostics. The
concrete gain is that the Scratchpad application/editor code remains unaware of
both Shirei and GPUI, and a second frontend can use the same semantic contract.

Gate 3 is therefore still deferred: if work continues, it should test only
bounded immutable visible-line resources, never whole-document snapshots. Gate
4 remains deferred until that result settles the document/editor ownership
question.

## Current verdict

**continue**

Continue only the narrow, framework-neutral experiment through Gate 3. Do not
publish, promise ABI stability, add widgets or universal serialization, or
route high-frequency editor mechanics through Caliber until bounded
cross-language data proves that the flexibility outweighs direct Shirei
integration.
