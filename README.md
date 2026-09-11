# Caliber (private experiment)

Caliber is a disposable experiment for a small native boundary between
application logic and presentation. It is not a framework, protocol, SDK, or
published crate. The code deliberately stays inside the Instar workspace so
it can be deleted if direct integration wins.

The experiment separates three bounded mechanisms:

- ordered control bytes;
- atomic revisioned state publications owned by the application;
- immutable resources, latest-value telemetry, and an optional bounded SPSC
  stream.

The core has no GUI, renderer, window, Wasm, async-runtime, or audio-engine
dependency. The provisional C ABI is only a foreign-boundary experiment; it
does not define application schemas. The Rust synthetic backend and Go cgo
smoke frontend use application-owned bytes to exercise the boundary.

## Run the current proof

```text
cargo fmt --manifest-path Cargo.toml --all -- --check
cargo test --manifest-path Cargo.toml
cargo clippy --manifest-path Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path experiments/synthetic-rust/Cargo.toml
cargo run --manifest-path experiments/synthetic-rust/Cargo.toml
python3 experiments/synthetic/trace_harness.py
```

Build the native library and run the foreign smoke test with cgo using the
commands in [`experiments/go-ffi/README.md`](experiments/go-ffi/README.md).

## Decision status

The experiment's final result is **NO-GO** for a library or public extraction:
the mechanisms are useful evidence, but there are no two independent real
consumers and the direct Rust integration is smaller for the only tested
application. See [`docs/EXPERIMENT-RESULTS.md`](docs/EXPERIMENT-RESULTS.md) for
the decision and stop conditions.

The design, ABI, ownership ledger, and falsification plan are in
[`docs/`](docs/). No compatibility promise or semver policy applies.
