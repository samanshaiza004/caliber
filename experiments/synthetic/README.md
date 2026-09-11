# Synthetic Caliber trace

This is a tiny, dependency-free model used to test the experimental design
without a GUI, Punks, audio, Wasmtime, or a foreign ABI. It deliberately does
not implement `caliber-core` and is not an API example.

Run:

```text
python3 trace_harness.py
```

The trace exercises:

- ordered control commands;
- atomic revisioned state publication;
- malformed-publication preservation;
- immutable bulk-resource references;
- latest-value telemetry with dropped intermediate values;
- direct-integration comparison data as an explicit result, not a slogan.

Its value is falsification: the model should make ownership and revision
mistakes obvious before a native or FFI implementation exists.
