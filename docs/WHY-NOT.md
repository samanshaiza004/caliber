# When not to use Caliber

Caliber is a narrow in-process native lifecycle boundary for application logic
and presentation. It is not an IPC protocol, serializer, GUI framework, or
distributed-object system. Choose an alternative when its scope better fits the
problem.

| Alternative | Prefer it when | What Caliber adds or changes |
| --- | --- | --- |
| Direct integration or ordinary FFI | One frontend and backend share a language/runtime, or one straightforward FFI boundary is easy to own. This is the default for a single-framework application. | Caliber standardizes bounded command backpressure, revisioned state publication, immutable read leases, generation-checked resource handles, wake and shutdown rules, and an append-only C ABI. In return it adds a runtime/library boundary, FFI glue, packaging/loading work, copying, and more lifecycle rules. |
| FlatBuffers | You need schema-defined binary data with generated accessors, schema evolution, and direct access to serialized buffers without first unpacking the entire structure. | FlatBuffers addresses representation and schema evolution; it does not supply Caliber's context lifecycle, bounded command queue, state publication, resource registry, wake, or shutdown contract. Use both only if each solves a distinct need. |
| Cap’n Proto serialization or RPC | You need schema-based data representation, or its RPC layer's object capabilities, promises, asynchronous calls, and broader peer communication model. | Cap’n Proto RPC is a real RPC and capability-lifecycle system. Caliber deliberately has a smaller in-process C ABI, bounded application-work handoff, latest-state/resource access, and explicit frontend-owned wake-thread shutdown. It is not a substitute for Cap’n Proto RPC. |
| JSON-RPC or an LSP-style protocol | Process isolation, transport flexibility, inspectable messages, request/response semantics, or reuse of existing protocol tooling matters more than direct in-process resource access. | Caliber avoids a separate message transport and exposes native leases and bounded in-process data planes. JSON-RPC/LSP can be easier to inspect and isolate; they may be the better boundary even when serialization costs are acceptable. |
| Tauri or another webview application toolkit | You want a complete application toolkit with a web-technology frontend and a native Rust host, and the system webview is a good product fit. | Tauri supplies a GUI/application structure and frontend-host communication model. Caliber does not choose a UI technology or provide windows, rendering, or widgets; it can serve a different native presentation boundary. These tools solve different layers and can coexist if needed. |

Do not choose Caliber just to make an application “polyglot” or to move bytes
through a C table. First write down the concrete boundary you expect to
replace, the ownership and shutdown problems you need standardized, and the
cost you accept for a foreign runtime and packaging.

## Measured costs and evidence

Caliber is not a zero-cost abstraction. Scratchpad's GPUI dogfood measured a
warm command-to-visible-resource path at 84.8 µs median and 118.1 µs p95 for a
9.6 KiB fixture (64 samples on an Apple M1). That end-to-end path includes Go
backend work, serialization, Caliber resource operations, and Rust handling;
it must not be quoted as Caliber-only latency. The optimized runtime has three
artifacts, and settled RSS was not measured. The direct Shirei path remains
simpler and has fewer copies and lifetime rules. See the full
[Scratchpad dogfood record](SCRATCHPAD-DOGFOOD.md) and
[experiment results](EXPERIMENT-RESULTS.md).

[Alicorn Scope](https://github.com/samanshaiza004/alicorn-scope) is a second
consumer with a Go backend and Odin/Alicorn frontend. Its
[Phase 1 design](https://github.com/samanshaiza004/alicorn-scope/blob/master/docs/phase-1.md)
describes the application-specific commands, state, and resources; its
[validation record](https://github.com/samanshaiza004/alicorn-scope/blob/master/docs/validation.md)
records a Windows native smoke and bounded application/parser measurements.
Those parser measurements explicitly exclude Caliber transport, so Scope is
evidence of a different integration and native packaging path, not a
Caliber-versus-direct benchmark. Scratchpad's
[GPUI dogfood record](https://github.com/samanshaiza004/scratchpad/blob/main/docs/history/GPUI-DOGFOOD.md)
provides the corresponding cross-language timing and artifact measurements.

Treat these dogfood results as evidence about specific consumers, platforms,
and workloads, not universal performance guarantees. The v0.1 release checklist
continues to require fresh-clone and external-developer validation.

## Sources for the alternatives

- [FlatBuffers overview](https://flatbuffers.dev/) and [tutorial](https://flatbuffers.dev/tutorial/)
- [Cap’n Proto RPC protocol](https://capnproto.org/rpc.html)
- [JSON-RPC 2.0 specification](https://www.jsonrpc.org/specification)
- [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
- [Tauri architecture](https://v2.tauri.app/concept/architecture/)
