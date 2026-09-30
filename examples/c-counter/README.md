# Canonical C ABI lifecycle example

[`lifecycle.c`](lifecycle.c) is the executable C example for Caliber ABI v1.
It includes the canonical [`include/caliber.h`](../../include/caliber.h),
loads a Caliber shared library, checks the versioned function table, creates a
context, dispatches and takes an application command, publishes and reads
state, and demonstrates resource handles and leases. It also starts and joins
a waiter, then stops and joins a second waiter before destroying the context.

The command, state bytes, resource bytes, and schema are application-defined
examples. Caliber does not interpret those payloads.

Build Caliber first with `cargo build -p caliber-ffi`. On Linux:

```sh
cc -std=c11 -Wall -Wextra -Werror -pthread -I include \
  examples/c-counter/lifecycle.c -ldl -o /tmp/caliber-c-lifecycle
/tmp/caliber-c-lifecycle "$PWD/target/debug/libcaliber_ffi.so"
```

On macOS, use the same command without `-ldl` and pass
`target/debug/libcaliber_ffi.dylib`.

From a Visual Studio Developer PowerShell on Windows:

```powershell
cl /nologo /std:c11 /W4 /WX /I include examples\c-counter\lifecycle.c `
  /Fe:caliber-c-lifecycle.exe
.\caliber-c-lifecycle.exe (Resolve-Path target\debug\caliber_ffi.dll)
```

The example is compiled and run against the freshly built library on Windows,
macOS, and Linux by the `fresh-consumer` CI job.
