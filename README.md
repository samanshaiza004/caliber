# Caliber

Caliber is a small native boundary between application logic and presentation,
allowing the application and UI to evolve independently and use different
languages and GUI frameworks.

Current status: **pre-v0.1 and being independently dogfooded**. The first v0.1
release is intended to make only the C ABI v1 stable, as defined in
[`docs/ABI.md`](docs/ABI.md). That compatibility promise begins with the first
v0.1 release; this pre-release branch may still revise the candidate ABI.
Caliber's Rust APIs, developer CLI, and dependency lock format remain
pre-1.0 and may change. The Rust crates are not published to crates.io, and
Caliber does not promise whole-project semantic-version compatibility or a
stable Rust SDK.

Caliber supplies bounded ownership and transport mechanisms. The application
still owns its meaning, schemas, policy, and domain state:

- **control** — ordered, bounded semantic commands and events;
- **state** — atomic revisioned publication of complete application state;
- **data** — immutable bulk resources, latest-value telemetry, and a Rust-core
  SPSC stream facility. Ordered streams are not part of the v0.1 foreign ABI.

Caliber is not a GUI framework, renderer, window system, audio engine, Wasm
runtime, async runtime, IPC protocol, universal object model, or application
schema. Foreign callers use the canonical [`include/caliber.h`](include/caliber.h)
and `caliber_get_api(1)` table. The planned v0.1 promise applies only to that
ABI, not to the `caliber-ffi` Rust API.

## Repository shape

The workspace contains:

- `caliber-core` — framework-neutral bounded mechanisms;
- `caliber-ffi` — the versioned C table over those mechanisms;
- `caliber` — a separate developer CLI for exact-Git source dependencies. It
  does not add dependency or build policy to `caliber-core`.
- `caliber-abi-tests` — C/Rust layout and frozen-client compatibility checks.

All Rust crates are unpublished and their Rust APIs are outside the v0.1 ABI
compatibility promise.

The synthetic Rust and Go/cgo experiments are deliberately retained. They
exercise the mechanisms with application-owned bytes without importing a GUI,
audio engine, or product schema.

## Run locally

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p caliber -- --help
cargo test --manifest-path experiments/synthetic-rust/Cargo.toml
cargo run --manifest-path experiments/synthetic-rust/Cargo.toml
python3 experiments/synthetic/trace_harness.py
```

The Go smoke frontend requires cgo and a locally built native library. See
[`experiments/go-ffi/README.md`](experiments/go-ffi/README.md).

The developer CLI lock format, sync/status/update/pin semantics, and project
validation hook are described in [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md).
The narrow v0.1 compatibility target and release gate are described in
[`docs/RELEASE-CHECKLIST.md`](docs/RELEASE-CHECKLIST.md).

The design, ABI and lifecycle notes, release checklist, ownership ledger,
experiment plan, results, and first
real application pressure test are in [`docs/`](docs/). The current evidence
and verdict are recorded in
[`docs/EXPERIMENT-RESULTS.md`](docs/EXPERIMENT-RESULTS.md) and
[`docs/SCRATCHPAD-DOGFOOD.md`](docs/SCRATCHPAD-DOGFOOD.md).

Licensed under either the MIT License or Apache License, Version 2.0, at your
option. See [`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).
