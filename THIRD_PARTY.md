# Third-party code

Code copied or adapted into this repository, with its licence. Dependencies pulled in through
Cargo, npm, Swift Package Manager or NuGet are not listed here; their manifests and the licence
checks in CI cover them. Model weights are listed in [docs/MODEL-WEIGHTS.md](docs/MODEL-WEIGHTS.md).

| Project | Licence | Where | Notes |
|---|---|---|---|
| [Handy](https://github.com/cjpais/Handy) by CJ Pais | MIT | `src-tauri/` (legacy 0.2 app) | Inkwell 0.2 was originally based on it. |
| [SQLite](https://sqlite.org) | Public domain | `core/crates/ink-store`, compiled in by `libsqlite3-sys` (`bundled` feature) | The C source ships inside `libsqlite3-sys` (MIT), so `cargo deny` sees only that crate's licence. The version is whichever one the locked `libsqlite3-sys` bundles. |
| [NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp) by NVIDIA | Apache-2.0 | `core/crates/ink-engines` (`engine-nemo`), linked as a shared library | Commit `97a15afa5caa9bce5baaa86c1184103877af4101`. Not vendored: `native/build-nemo-speech.sh` builds it from a checkout, and the adapter's C declarations follow its `nemo_speech/diar.h`. |
| [ggml](https://github.com/ggml-org/ggml) | MIT | Built with NeMo-Speech.cpp, as its own shared libraries | NeMo's submodule, commit `c03b4e2bcece5134827881af90242086daf75be5`, with NeMo's `ggml-patches/` applied. |
| [SentencePiece](https://github.com/google/sentencepiece) by Google | Apache-2.0 | Linked by NeMo-Speech.cpp | Version 0.2.2. Its library carries protobuf-lite, Darts-clone and its own copy of Abseil. NeMo's notices give their licences only as "Apache 2.0 and BSD". Neither they nor any source on the build machine names the BSD variant; confirm it from the SentencePiece 0.2.2 sources. |
| [Abseil](https://github.com/abseil/abseil-cpp) by Google | Apache-2.0 | Linked by NeMo-Speech.cpp | Version 20260817.0. |

## Rules

- Allowed licences for code: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib.
- Code adapted from a BSD-licensed project keeps its notice in the file header as well as a row here.
- Native libraries built from source (C and C++ dependencies of an engine) get a row here when their
  build lands, because `cargo deny` does not see them.
