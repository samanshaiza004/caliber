# Fresh consumer fixture

This small project pins the Caliber v0.1 candidate merge commit. The CI job
copies the lock and diagnostics config to a new temporary consumer directory,
uses the Caliber CLI to materialize the exact Git revision, builds the pinned
`caliber-ffi` library, then runs `doctor`, `check`, and the C and Python
lifecycle consumers from that checkout.

The CI test intentionally does not point at the source checkout or reuse a
managed dependency directory. It exercises the same exact-revision path a
consumer gets from a clean clone.
