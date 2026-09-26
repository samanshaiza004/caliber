# Caliber v0.1 release checklist

The v0.1 compatibility promise is intentionally narrower than a whole-project
stability promise.

## Stable surface

The first v0.1 release promises compatibility only for the native C ABI v1:

- `caliber_get_api(1)` version negotiation and `CaliberApiV1` layout;
- public C record layouts, fixed-width status values, and existing field
  meanings;
- append-only function-table extensions checked through `struct_size`;
- command/state/resource/telemetry behavior and ownership;
- wake, lease, and shutdown semantics documented in `ABI.md` and
  `LIFECYCLE.md`.

The promise is for an in-process native caller using the same pointer width as
the library. It is not a wire-format or cross-architecture compatibility
promise.

`caliber-core`, the Rust API of `caliber-ffi`, the `caliber` CLI, its lock
schema, and application-owned protocols remain pre-1.0 and may change. The
Rust crates are not published to crates.io. This ABI commitment does not imply
whole-project Semantic Versioning stability.

## Before tagging v0.1.0

- [ ] Complete Caliber issue #8 with at least one external developer. Record
  setup blockers, API/ownership mistakes, platform outcomes, and time spent on
  Caliber-only glue. Do not substitute maintainer testing for this gate.
- [ ] Confirm Scratchpad and Scope use exact lock revisions and both pass
  candidate validation plus their native lifecycle smoke on supported hosts.
- [ ] Run `cargo fmt --all -- --check`, workspace Clippy with warnings denied,
  workspace build, and workspace tests.
- [ ] Pass ABI conformance and compatibility tests: C/Rust sizes and offsets,
  status values, function-table prefix, frozen old v1 client, and current v1
  client against the candidate library.
- [ ] Pass the Caliber CI matrix on every supported operating system and
  architecture. Native GUI smoke is required where the host has a presentable
  surface; headless checks do not stand in for it.
- [ ] Run `caliber doctor` and `caliber check` in both consumer projects.
- [ ] Review every public header change. Within ABI v1, only compatible
  append-only table extensions are allowed. A changed existing layout,
  function signature, status meaning, ownership rule, or wake/shutdown rule
  requires a new `CaliberApiV2` and `caliber_get_api(2)` while retaining v1 for
  supported v0.1.x consumers.
- [ ] Reconcile the README, ABI guide, lifecycle guide, and release notes with
  the exact promise above. Do not describe the Rust crates or developer tools
  as stable.
- [ ] Tag and describe the release as a stable C ABI v1 with pre-1.0 Rust APIs
  and developer tooling. Do not publish the Rust crates as part of this gate.

## After v0.1.0

For each v0.1.x release, rerun the ABI compatibility harness and supported
consumer checks. Never modify the frozen old-client fixture to match a newer
ABI; add a new fixture for a new compatibility promise. Breaking ABI work is a
separate v2 addition and migration plan, not a v1 edit.
