# Illustrative Python `ctypes` consumer

[`lifecycle.py`](lifecycle.py) shows a dependency-free third-language consumer
calling Caliber's C ABI v1. It locally mirrors the required C records and
function-table slots to demonstrate context creation, command transport,
state publication and leases, wake observation, and shutdown. The longer C
example also covers resource handles and mapped leases.

This script is illustrative, **not a supported Python SDK or maintained
binding**. Its mirrored declarations are deliberately consumer-side code and
must not replace or be copied over Caliber's canonical
[`include/caliber.h`](../../include/caliber.h).

Build Caliber, then run the script with the library path. For example:

```sh
cargo build -p caliber-ffi
python3 examples/python-ctypes/lifecycle.py "$PWD/target/debug/libcaliber_ffi.so"
```

Use `target/debug/libcaliber_ffi.dylib` on macOS or
`target/debug/caliber_ffi.dll` on Windows. The example is exercised in the
Linux and macOS `experiments` CI job.
