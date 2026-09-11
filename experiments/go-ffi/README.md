# Go FFI smoke frontend

This is a deliberately non-GUI foreign frontend for the experimental Caliber
C ABI. It uses only the versioned API table and treats command payloads as
opaque bytes. The smoke test proves that a Go caller can:

- reject an unsupported ABI version and inspect the v1 table;
- create and destroy a context through function pointers;
- dispatch empty, non-empty, null, and oversized command inputs;
- observe the wake sequence change only after accepted work;
- publish and lease state and immutable resources;
- publish and read latest-value telemetry;
- release leases and owner references, including empty/null cleanup.

The publish operations are deliberately opaque: the frontend chooses the
payload bytes, schema, and telemetry values, while the ABI owns revision,
generation, lease, bound, and wake behavior.

## Run

From this directory on macOS or Linux:

    cargo build --manifest-path ../../Cargo.toml -p caliber-ffi
    CGO_LDFLAGS="-L../../target/debug" DYLD_LIBRARY_PATH=../../target/debug go run .

Expected output:

    caliber Go FFI smoke: PASS
    ABI v1, wake sequence 2, command bytes bounded at 4

The exact wake sequence can differ if the ABI gains another accepted event;
the test checks monotonic change and preservation on rejected input rather
than depending on a fixed event count.

The command requires cgo and a native Rust library. Windows needs the
corresponding Cargo target output and DLL search path configuration; this
experiment does not add a platform-specific GUI or build system.
