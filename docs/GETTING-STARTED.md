# Getting started

This guide is a short route from a fresh Caliber checkout to a working
consumer. The supported foreign contract for the v0.1 candidate is the C ABI
in [`include/caliber.h`](../include/caliber.h). Application command and state
payloads remain the application's own schema.

## Prerequisites

- Git
- Rust stable with Cargo (Rust 1.85 or newer for edition 2024)
- A C compiler supported by the Rust target
- Python 3 for the optional `ctypes` example

On Windows, build from a Visual Studio Developer PowerShell with the Windows
SDK available. The C example's README gives direct `cc` and `cl` build commands.

## Build and verify Caliber

From the repository root:

```sh
cargo build -p caliber-ffi
cargo test --workspace
cargo run -p caliber -- --help
```

The workspace tests include C/Rust ABI layout and compatibility checks. The
`caliber doctor` and `caliber check` commands inspect a configured consumer
project, not the Caliber source checkout. Once a consumer has a
`dependencies.lock.json`, a `caliber.config.json`, and a built Caliber library,
run those commands from that project or pass `--project-root DIR`. `doctor`
checks dependency/tool/library setup; `check` also exercises the ABI boundary.
See the [debugging guide](DEBUGGING.md) for details and failure handling.

## Run the examples

The [canonical C counter](../examples/c-counter/README.md) follows one command
from dispatch through application processing, state publication and wake,
then reads a leased snapshot and shuts down in the required order. Its README
provides tested compiler commands using the repository's public header and
library.

The [Python `ctypes` counter](../examples/python-ctypes/README.md) demonstrates
the same contract using only Python's standard library. Its declarations are
example code, not a supported Python SDK; `include/caliber.h` remains the
contract.

## Understand the boundary

Read [the lifecycle guide](LIFECYCLE.md) before adapting the examples. In
particular, keep command/state schemas application-owned, release every lease
exactly once, stop and join the frontend wake waiter, then destroy the
context. Caliber does not require a periodic polling timer.

Caliber remains experimental before v0.1. The intended v0.1 compatibility
promise covers the C ABI v1 only; see the [release checklist](RELEASE-CHECKLIST.md).
