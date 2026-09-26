# Scratchpad dogfood

Status: Gates 1–4 edit-spike complete on the experimental `gpui-dogfood` branch.
Caliber remains pre-v0.1 and is independently dogfooded. Its intended v0.1
compatibility promise covers only C ABI v1 beginning with the first v0.1
release; Rust APIs and developer tooling remain pre-1.0. This document records
what the first real application changed and what the bounded data experiment
actually measured.

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
as a dependency to Scratchpad's root module. The `gpui-dogfood` branch adds a
nested cgo backend and Rust/GPUI shell that implement the same semantic
interface through the real Caliber foreign boundary.

Gate 3 adds only a bounded visible-line request. The Go adapter reads directly
from the existing piece-backed buffer, publishes a 48-byte-header `SPVS`
immutable resource with at most 256 lines and 64 KiB of payload, and Rust maps,
validates, copies, and releases it. The complete document never crosses this
interface. Gate 4 adds only the bounded editable spike below; it does not port
or rewrite Scratchpad's editor.

Gate 3.5 replaces the initial per-line extraction loop with one bounded
contiguous piece-range copy. The foreign path now reports warm median/p95
timings over 64 samples: 84.8/118.1 µs for the 9.6 KiB fixture, with the
backend pump containing Go decode/extraction/resource publication and response
JSON. The direct extraction benchmark is 37.7 µs, 9,472 bytes, and one
allocation. A visible slice is accepted by the GPUI model only when its
document id, application revision, and editor revision match current state;
100 rapid pending ranges collapse to the latest request.

## Gate 4 edit spike

The first editable path tests the Model B hypothesis without moving the
scalable editor into Rust. The application remains authoritative for complete
document bytes, editor revisions, undo/domain edit semantics, dirty state, and
persistence. The Rust side owns only one bounded visible window plus local
caret and selection state. It sends a `replace_document` intent with the
document id, application revision, expected editor revision, global byte range,
and bounded replacement bytes.

The visible resource descriptor carries the window's global `start_byte`, so a
local Rust selection can map back to the authoritative Go document without
serializing the complete document or guessing through replacement characters.
The first Rust session accepts only a non-truncated valid-UTF-8 window. This is
a temporary mapping constraint in the spike; the Go wire and Scratchpad editor
remain byte-oriented and continue to preserve arbitrary source bytes.

Rust applies an edit optimistically to its bounded local copy. Go accepts it
only when the editor revision still matches, applies the existing document
replacement operation, publishes the new state, and returns a structured
acknowledgement. A stale edit is rejected without mutation, and the local
session can roll back its optimistic copy. The real foreign smoke verifies
optimistic bytes, acknowledgement, stale rejection, save, on-disk bytes, and
clean shutdown through the actual Caliber ABI.

High-frequency cursor motion, selection updates, viewport movement, IME
preedit, shaping, layout, folds, projections, and paint remain frontend-local
or deferred. The spike permits only one in-flight edit; batching and typing
coalescence are deferred until a real interactive editor is justified. Gate 4
is a seam test, not a complete GPUI editor.

## What stays frontend-local

The boundary does not carry keystrokes, cursor movement, selections, text
buffers, viewport scroll, shaping, row maps, folds, projections, or paint
commands. Those are high-frequency editor/presentation mechanics and remain
local to each frontend: the existing Shirei path keeps its full editor, while
the GPUI spike keeps only one bounded optimistic session. Serializing them
would erase the scale work that Scratchpad already completed and would turn
Caliber into a request-per-property protocol.

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
| Flexibility gained | Shirei is the only shell path | A typed shell contract now consumed by both Shirei and the experimental Rust/GPUI shell |

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

The Gate 3.5 foreign test adds a separate measurement on the development Apple
M1 host: approximately 25.8/27.9 µs median/p95 per command → state read and
84.8/118.1 µs median/p95 per command → bounded visible resource → Rust cache
round trip, using 64 warm samples with the release-built Go backend and
Caliber library. The optimized runtime consists of an 18,607,488-byte Rust
executable, a 24,223,696-byte Go c-shared backend, and a 428,600-byte Caliber
library: three runtime artifacts. The Rust foreign test is run through Cargo's
normal test target, so these timings are engineering measurements rather than
an all-release performance claim. Settled RSS was not sampled, and the
managed macOS environment could not complete the native window smoke within
its 30-second bound. Gate 4 adds no whole-document copy: its optimistic
rollback copy is limited to the same bounded Rust window, and Go uses the
existing piece-backed document replacement operation. The direct Shirei path
remains simpler and has fewer copies, artifacts, and lifetime rules.

## Result

The slice answers the critical compatibility question positively: the current
Shirei UI and the experimental GPUI shell can use a small application-owned
semantic contract without making Scratchpad's domain package know about either
GUI framework or disturbing the scalable editor. Gate 4 adds evidence for a
source-edit seam, but not for a complete editor split: it currently excludes
IME, viewport/layout ownership, and arbitrary-byte source-position mapping in
Rust. The direct Go adapter remains cheaper than crossing the Caliber C ABI,
and the foreign path adds measurable copies, artifacts, and lifetime rules.

This is evidence to **continue**, not to generalize. Keep Caliber's three-plane
mechanisms and ownership tests independent; do not add editor serialization,
widget abstractions, IPC, or an async runtime. Any larger editor experiment
should proceed only after reviewing whether this bounded source-edit seam earns
its complexity.
