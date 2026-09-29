# Caliber

Caliber is a small native lifecycle boundary for polyglot desktop applications.
It coordinates bounded commands, revisioned state, leased immutable resources,
generation-safe handles, coalescing wakeups, and explicit shutdown through a C
ABI.

**Your application owns the schema and meaning. Caliber owns the boundary
mechanics.** Caliber does not own application policy, domain state, or
presentation.

## When Caliber fits

Caliber is useful when an application needs to keep its domain and presentation
independent across language or GUI-framework boundaries, and needs an explicit
contract for ownership, bounded work, state publication, resource lifetime,
notification, and shutdown. It is an in-process native ABI, not IPC.

If the application and UI live comfortably in one language and framework, and
there is no real need to replace the presentation boundary, prefer direct
integration. It is simpler, cheaper, and avoids foreign-runtime and packaging
costs. Caliber adds FFI glue, library packaging/loading, ownership rules, and
some bounded copying; it is not a free abstraction.

## Lifecycle at a glance

```text
Frontend                         Application
   |                                 |
   | dispatch(command)               |
   +-----------> Caliber ----------->|
   |                                 | process command
   |                  publish_state()|
   |<------ wake ----- Caliber <-----+
   |                                 |
   | read_latest_state()             |
   +-----------> Caliber             |
   |<------ immutable lease          |
   |                                 |
   | render                          |
   | release(lease)                  |
   +-----------> Caliber             |
```

Commands are bounded and copied; state publications are atomic and revisioned;
resources are immutable and generation-checked; wakeups may coalesce; leases
have explicit release; and shutdown has an explicit order. Caliber does not
select a serializer or define the payload schemas.

## Cost and dogfood evidence

The boundary has measurable costs. In the Scratchpad GPUI dogfood, a warm
command-to-visible-resource round trip over a 9.6 KiB fixture measured 84.8 µs
median and 118.1 µs p95 on an Apple M1 (64 samples). The path includes Rust
dispatch, Go command handling and resource publication, response decoding, and
Rust resource mapping/copying; it is not a measurement of Caliber alone. The
optimized runtime consisted of a Rust executable, Go shared backend, and
Caliber library. Settled RSS was not sampled. Read the
[Scratchpad dogfood measurements](docs/SCRATCHPAD-DOGFOOD.md) and
[experiment results](docs/EXPERIMENT-RESULTS.md), plus Scratchpad's
[GPUI dogfood report](https://github.com/samanshaiza004/scratchpad/blob/main/docs/history/GPUI-DOGFOOD.md),
for the scope and limitations of those numbers. Alicorn Scope provides a
second Go-to-native-frontend consumer; its
[validation record](https://github.com/samanshaiza004/alicorn-scope/blob/master/docs/validation.md)
separates application/parser measurements from Caliber transport costs.

## Five-minute orientation

Start with the [getting-started guide](docs/GETTING-STARTED.md), then build the
canonical C example and the third-language example. The C header is the
foreign-language contract: [`include/caliber.h`](include/caliber.h).

## Stability

Caliber is pre-v0.1. The first v0.1 release is intended to stabilize only the
append-only C ABI v1 described in [`docs/ABI.md`](docs/ABI.md); that promise
begins with the release, and this candidate may still change before then.
Caliber's Rust APIs, developer CLI, dependency lock format, and application
payload schemas remain pre-1.0 and may change. Rust crates are not published to
crates.io. This is not a promise of whole-project semantic-version stability.

## Repository shape

- `caliber-core` contains framework-neutral bounded mechanisms.
- `caliber-ffi` implements the versioned C table over those mechanisms.
- `caliber` is a developer CLI for exact-Git source dependencies; it does not
  add dependency or build policy to `caliber-core`.
- `caliber-abi-tests` checks C/Rust layout and frozen-client compatibility.
- `examples/` contains small consumers of the public ABI.

The synthetic Rust and Go/cgo experiments exercise the mechanisms with
application-owned bytes without importing a GUI, audio engine, or product
schema.

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

## Documentation

- [Getting started](docs/GETTING-STARTED.md)
- [ABI v1 and compatibility](docs/ABI.md)
- [Lifecycle and shutdown](docs/LIFECYCLE.md)
- [Serialization and data-plane choices](docs/SERIALIZATION.md)
- [Errors and debugging](docs/DEBUGGING.md)
- [When not to use Caliber](docs/WHY-NOT.md)
- [Ownership and threading](docs/OWNERSHIP.md)
- [Developer dependency CLI](docs/DEPENDENCIES.md)
- [v0.1 release checklist](docs/RELEASE-CHECKLIST.md)
- [Scratchpad dogfood](docs/SCRATCHPAD-DOGFOOD.md)
- [Experiment results](docs/EXPERIMENT-RESULTS.md)

Licensed under either the MIT License or Apache License, Version 2.0, at your
option. See [`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).
