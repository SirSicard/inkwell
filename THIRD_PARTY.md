# Third-party code

Code copied or adapted into this repository, with its licence. Dependencies pulled in through
Cargo, npm, Swift Package Manager or NuGet are not listed here; their manifests and the licence
checks in CI cover them. Model weights are listed in [docs/MODEL-WEIGHTS.md](docs/MODEL-WEIGHTS.md).

| Project | Licence | Where | Notes |
|---|---|---|---|
| [Handy](https://github.com/cjpais/Handy) by CJ Pais | MIT | `src-tauri/` (legacy 0.2 app) | Inkwell 0.2 was originally based on it. |
| [SQLite](https://sqlite.org) | Public domain | `core/crates/ink-store`, compiled in by `libsqlite3-sys` (`bundled` feature) | The C source ships inside `libsqlite3-sys` (MIT), so `cargo deny` sees only that crate's licence. The version is whichever one the locked `libsqlite3-sys` bundles. |

## Rules

- Allowed licences for code: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib.
- Code adapted from a BSD-licensed project keeps its notice in the file header as well as a row here.
- Native libraries built from source (C and C++ dependencies of an engine) get a row here when their
  build lands, because `cargo deny` does not see them.
