# Caliber

Caliber is an experimental small native boundary between application logic and
presentation, allowing the application and UI to evolve independently and
potentially use different languages and GUI frameworks.

Current status: **experimental, being independently dogfooded, no stable API
yet**. The repository is intentionally small and may change or be deleted if
direct integration wins. It is not published to crates.io and does not promise
ABI stability, semantic-version compatibility, or a supported SDK.

Caliber supplies bounded ownership and transport mechanisms. The application
still owns its meaning, schemas, policy, and domain state:

- **control** — ordered, bounded semantic commands and events;
- **state** — atomic revisioned publication of complete application state;
- **data** — immutable bulk resources, latest-value telemetry, and bounded
  ordered streams.

Caliber is not a GUI framework, renderer, window system, audio engine, Wasm
runtime, async runtime, IPC protocol, universal object model, or application
schema. `caliber-ffi` is a provisional C-compatible experiment for foreign
callers; it is not an ABI commitment.

## Repository shape

Only two crates are kept:

- `caliber-core` — framework-neutral bounded mechanisms;
- `caliber-ffi` — the provisional versioned C table over those mechanisms.

The synthetic Rust and Go/cgo experiments are deliberately retained. They
exercise the mechanisms with application-owned bytes without importing a GUI,
audio engine, or product schema.

## Run locally

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --manifest-path experiments/synthetic-rust/Cargo.toml
cargo run --manifest-path experiments/synthetic-rust/Cargo.toml
python3 experiments/synthetic/trace_harness.py
```

The Go smoke frontend requires cgo and a locally built native library. See
[`experiments/go-ffi/README.md`](experiments/go-ffi/README.md).

The design, ABI notes, ownership ledger, experiment plan, results, and first
real application pressure test are in [`docs/`](docs/). The current evidence
and verdict are recorded in
[`docs/EXPERIMENT-RESULTS.md`](docs/EXPERIMENT-RESULTS.md) and
[`docs/SCRATCHPAD-DOGFOOD.md`](docs/SCRATCHPAD-DOGFOOD.md).

Licensed under either the MIT License or Apache License, Version 2.0, at your
option. See [`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).
