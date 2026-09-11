# Rust synthetic backend

This is the smallest native Caliber falsification harness. It is deliberately
not Punks, a GUI, a renderer, an FFI adapter, or a proposed SDK. The sample
library and all command/state schemas are local to this experiment.

The backend exercises the three Caliber planes through the real
`caliber-core` mechanisms:

- ordered search, selection, preview, stop, and favorite commands;
- an atomically replaced revisioned state publication;
- a bounded Postcard state fixture with fixed-width favorite and resource
  reference fields;
- one immutable waveform resource referenced by state rather than copied into
  each publication or repaint;
- a fixed-width latest-value meter where intermediate readings may be
  overwritten;
- deterministic command traces, including application-owned rejection of
  stale revisions.

Run the tests and the tiny trace driver from the Caliber directory:

```text
cargo test --manifest-path experiments/synthetic-rust/Cargo.toml
cargo run --manifest-path experiments/synthetic-rust/Cargo.toml
```

## Why this is a falsification harness

The experiment is successful only if it makes a direct-integration comparison
worth doing. A direct Rust application could keep the same sample state,
commands, waveform bytes, and meter in one module with no boundary at all.
That baseline should be compared with this backend on:

- command and publication boilerplate;
- duplicated domain models and byte encoding;
- state-to-frontend and command-to-backend latency;
- resource lookup and memory ownership complexity;
- idle work and allocation behavior;
- debugging and shutdown failure modes.

The Caliber side must earn its cost by showing a concrete benefit: the same
backend contract can be consumed by independent frontends, backend traces run
without a GUI, and bulk waveform data and high-rate meters avoid snapshot
serialization. If the direct version is clearly simpler with no material loss
of flexibility, the result is **NO-GO** and this experiment should be deleted.

Passing these tests is therefore evidence for mechanisms, not evidence that a
public Caliber library should exist.
