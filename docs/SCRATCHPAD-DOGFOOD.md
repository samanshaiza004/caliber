# Scratchpad dogfood

Status: first boundary slice complete; Caliber remains experimental and is
being independently dogfooded. This document records what the first real
application changed, including where it did not justify using Caliber yet.

## Application

[Scratchpad](https://github.com/samanshaiza004/scratchpad/tree/caliber-dogfood)
is a native Go, file-first editor whose current shell uses Shirei. Its
`application` package
already contains the useful ownership boundary: it owns workspace and document
identity, open-document order, active selection, save/conflict policy, and
document lifecycle. The `ui` package owns Shirei composition and transient
presentation state.

The first slice adds a small `application.PresentationClient` contract. Its
semantic commands are:

- open a path;
- select an open document;
- save a document;
- close a document with an explicit discard decision.

Its snapshot contains only shell-visible state: lifecycle revision, workspace
root, active document id, document id/path, dirty/status state, content
revision, and detected language. The `application` package imports no Shirei.
The existing Shirei command paths for open, select, save, and close dispatch
through the contract, while the rest of the shell remains unchanged.

The local Scratchpad working tree contains this proof in:

- `application/presentation.go`;
- `application/presentation_test.go`;
- the semantic command routing in `ui/root.go`.

The proof is also available as commit
[`8bfa8a6`](https://github.com/samanshaiza004/scratchpad/commit/8bfa8a6)
on the `caliber-dogfood` branch. It is intentionally a branch, not a rewrite
of Scratchpad's main line.

The existing editor-scale work was not rewritten and no Caliber crate was added
as a dependency. The contract is intentionally a direct Go adapter today. A
future Caliber-backed client can implement the same semantic interface after a
real cross-language cost is justified.

## What stays frontend-local

The boundary does not carry keystrokes, cursor movement, selections, text
buffers, viewport scroll, shaping, row maps, folds, projections, or paint
commands. Those are high-frequency editor/presentation mechanics and remain in
Scratchpad's existing editor and Shirei path. Serializing them would erase the
scale work that Scratchpad already completed and would turn Caliber into a
request-per-property protocol.

The application remains authoritative for file identity, document order,
active selection, dirty/conflict status, save policy, and lifecycle. The
frontend remains authoritative for layout, input interpretation, shaping,
scroll/fold view state, and rendering. This is the smallest boundary that lets
a second shell later ask the same application questions without knowing
Shirei.

## Direct integration comparison

The direct integration baseline is the current Scratchpad architecture: the
Shirei shell receives the Go `*application.Application` directly and the
application owns the editor objects. The dogfood slice deliberately measures
the extra contract, not unrelated GUI runtime cost.

| Cost or benefit | Direct Shirei integration | First semantic contract slice |
| --- | --- | --- |
| Scratchpad glue added | 0 lines | 129-line contract, 124-line test, plus small routing and revision hooks |
| Command encoding | Go method call | Go struct + interface dispatch; no serialization |
| State snapshot | Direct field access | One copied summary slice; no document bytes |
| Buffer copies per repaint | 0 from this seam | 0; editor remains local |
| Allocation after setup | Existing UI/application behavior | Snapshot allocates its summary slice; dispatch itself is allocation-free in the tested path |
| Command/state latency | Baseline method call and existing frame work | Local Go interface/snapshot cost; no FFI or wire latency was claimed |
| RSS/binary delta | Baseline | Not isolated reliably on macOS for this small in-process slice; no native Caliber library was linked |
| Debugging | One concrete application object | One semantic adapter plus a revision field; errors remain application errors |
| Shutdown/lifetime | Existing Scratchpad lifecycle | Unchanged; no foreign handles, callback, or new goroutine introduced |
| Flexibility gained | Shirei is the only shell path | A typed shell contract and a testable headless consumer shape; a second frontend is still unbuilt |

The standalone Caliber mechanisms have a different cost profile: control
dispatch copies bounded bytes, state publication copies a bounded immutable
payload, resource publication copies once and mapped reads hold an immutable
lease, telemetry uses a fixed-width latest slot, and the SPSC stream is
preallocated. Those costs are documented in `DESIGN.md`, `OWNERSHIP.md`, and
the ABI notes, but were not falsely attributed to Scratchpad because this first
slice does not link the Rust FFI library.

The two standalone crates contain 2,775 raw Rust source lines including their
unit tests and documentation comments. As a reference point rather than a
Scratchpad delta, an Apple M1 release build produced a 419 KiB dynamic library
and a 17 MiB static library. The first dogfood did not link either artifact, so
incremental Scratchpad RSS and binary size remain unmeasured.

The command/state latency and allocation numbers below are intentionally
machine-specific smoke evidence, not API targets. On the development Apple M1
host, three benchmark runs reported approximately 9.5–10.3 ns/op and 0 B/op
for semantic select dispatch, versus 9.6–13.1 ns/op and 0 B/op for direct
`Activate`; a one-document snapshot reported 48–56 ns/op, 64 B/op, and one
allocation. Re-run the Scratchpad checkout's normal commands when comparing
future clients:

```text
env GOCACHE=/private/tmp/scratchpad-go-cache go test ./...
go test ./application -run '^TestPresentationContract'
go test ./application -bench 'BenchmarkPresentation' -benchmem
```

The full suite passed with the first slice. These numbers measure only the
local Go contract and are not evidence about Rust/FFI latency. The important
editor-path facts are zero editor copies, no new goroutine, and one
summary-slice allocation per published snapshot.

## Result

The slice answers the critical compatibility question positively: the current
Shirei UI can use a small application-owned semantic contract without making
Scratchpad's domain package know about Shirei or disturbing the scalable
editor. It also exposes the main limitation: a direct Go adapter is currently
cheaper than crossing the provisional C ABI, and no second real frontend has
yet demonstrated reuse.

This is evidence to **narrow**, not to generalize. Keep Caliber's three-plane
mechanisms and ownership tests independent; do not add editor serialization,
widget abstractions, IPC, or an async runtime. The next experiment should only
introduce a foreign Caliber client if a concrete second frontend makes the
additional copies, latency, lifetime rules, and debugging cost measurable.
