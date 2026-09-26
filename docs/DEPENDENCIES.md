# Exact Git source dependencies

`caliber` is a developer tool for projects that consume native source trees
across language boundaries. It materializes direct Git dependencies at exact
commits; it is not a package manager or build system.

## Lock file

Run commands from a project that has `dependencies.lock.json`, or pass
`--project-root DIR` after the command. The lock keeps Scope's original shape:

```json
{
  "schema": 1,
  "alicorn": {
    "repository": "https://github.com/example/alicorn.git",
    "ref": "main",
    "revision": "0123456789abcdef0123456789abcdef01234567"
  }
}
```

The dependency name is the object key. `ref` is optional and is consulted only
by `update` (and as a fetch fallback when a locked object is not present).
`revision` is always a full Git commit ID. A project's managed checkouts live
at `.deps/<dependency-name>`.

## Commands

```text
caliber sync
caliber status
caliber update alicorn
caliber pin alicorn ../alicorn
```

- `sync` checks out the revisions already in the lock. It never changes the
  lock. If the object is already in a clean managed checkout, it does not
  contact the remote. It fetches only when that object is unavailable. A dirty
  managed checkout is left untouched and rejected.
- `status` reports locked revision, managed path, local HEAD, clean/dirty, and
  synchronization state. It does not contact remotes.
- `update NAME` fetches NAME's configured tracked ref, materializes its
  candidate commit, runs the project's validation hook, then atomically writes
  the lock. One dependency is changed per invocation.
- `pin NAME CHECKOUT` reads the clean developer-owned checkout's HEAD, proves
  that the configured repository can materialize that commit, runs the same
  validation hook, then atomically writes the lock. The supplied checkout is
  never changed.

Developer-owned overrides can be used without mutation:

```text
caliber sync --override alicorn=../alicorn
caliber sync --override alicorn=../alicorn --allow-dirty-overrides
```

The first form requires a clean checkout at the locked revision and matching
repository origin. The explicit development form accepts a different or dirty
checkout and warns that the build is not reproducible from the lock.

## Project validation hook

Projects that allow `update` and `pin` provide a small `caliber.config.json`:

```json
{
  "schema": 1,
  "validation": {
    "alicorn": {
      "windows": ["pwsh", "-NoProfile", "-File", "tools/validate-candidate.ps1"],
      "unix": ["sh", "tools/validate-candidate.sh"]
    }
  }
}
```

Commands are argument arrays, never shell strings. The process runs from the
project root and receives `CALIBER_PROJECT_ROOT`, `CALIBER_DEPENDENCY`,
`CALIBER_CANDIDATE_REVISION`, and `CALIBER_CANDIDATE_ROOT`. Arguments may also
contain `{candidate}`, `{revision}`, or `{dependency}` placeholders. A missing
hook or nonzero result prevents the lock change. Caliber does not know how to
build Odin, Go, Rust, a GUI, or any particular application.

Projects bootstrap the Caliber CLI separately from their dependency graph.
The bootstrap pin identifies the source used to build the CLI itself; runtime
dependencies and their revisions remain solely in `dependencies.lock.json`.
