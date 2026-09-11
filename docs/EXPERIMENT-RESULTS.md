# Caliber experiment results

Final verdict: **NO-GO** for a Caliber library or public extraction. This is
an experimental result, not a release recommendation. The implementation is
kept as disposable evidence only.

## Evidence completed

- `caliber-core` provides bounded control, revisioned state, generation-checked
  immutable resources, latest-value telemetry, SPSC overflow behavior, and a
  coalescing wake signal.
- The Rust synthetic backend exercises all three planes with application-owned
  schemas, a single waveform resource, stale-command policy, malformed-state
  preservation, and deterministic trace replay.
- The provisional versioned C table has explicit pointer/length checks,
  bounded queues, lease cleanup, generation errors, short-config handling,
  panic containment, and Rust round-trip tests.
- The Go cgo smoke frontend consumes the same table and exercises command
  dispatch/peek/take, state publication/read, immutable resource mapping and
  release, latest telemetry, wake observation, and cleanup.
- The idle queue test demonstrates that the core does not require a polling
  timer merely to wait for control work.

Commands used for the current evidence:

```text
cargo fmt --manifest-path caliber/Cargo.toml --all -- --check
cargo test --manifest-path caliber/Cargo.toml
cargo clippy --manifest-path caliber/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path caliber/experiments/synthetic-rust/Cargo.toml
cargo run --manifest-path caliber/experiments/synthetic-rust/Cargo.toml
python3 caliber/experiments/synthetic/trace_harness.py
cargo build --manifest-path caliber/Cargo.toml -p caliber-ffi
(cd caliber/experiments/go-ffi && CGO_LDFLAGS="-L../../target/debug" DYLD_LIBRARY_PATH=../../target/debug go run .)
```

The final command requires cgo and a native library build; the exact macOS
invocation is documented beside the smoke frontend.

## Evidence deliberately missing

- There is no second real GUI frontend. No Fenestra or Shirei checkout is
  available in this workspace, so no substitute GUI framework was invented.
- There is no Punks facade yet. The sibling Punks checkout is independently
  dirty and was not modified; integrating it before the synthetic proof would
  couple the experiment to a large presentation/runtime dependency.
- No benchmark numbers are advertised. A direct-integration kill test still
  needs measured boilerplate, latency, allocations, RSS, and debugging/
  shutdown cost for a real application.
- The C table is not stable and has no generated public header or bindings.
  It is a seam to falsify, not a package to publish.

## Evaluation

The language split is technically possible: the Go smoke caller consumed the
same C table as the Rust tests. That is not evidence of a useful UI split,
because it has no GUI and there is no second real frontend. GUI-framework
neutrality, waveform repaint behavior, and application-level debugging cost
therefore remain unproven.

For the only real consumer, the direct Rust synthetic application is plainly
smaller than carrying a 2,600-line core/ABI boundary plus lease and pointer
contracts. The boundary buys no demonstrated reuse until a second independent
frontend exists. The required Punks facade, Fenestra frontend, second
foreign GUI, latency/RSS/binary measurements, and foreign stream/wake adapter
were intentionally not invented to rescue that result.

**NO-GO.** Do not publish or split these mechanisms into repositories. Keep
the bounded ownership and revision tests as design evidence; use direct
application integration until a concrete independent consumer makes the
trade-off measurable. Do not add widgets, layout, rendering, universal
serialization, IPC, or an async runtime to reverse this decision.
