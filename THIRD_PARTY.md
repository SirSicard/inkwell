# Third-party code

Code copied or adapted into this repository, with its licence. Dependencies pulled in through
Cargo, npm, Swift Package Manager or NuGet are not listed here; their manifests and the licence
checks in CI cover them. Model weights are listed in [docs/MODEL-WEIGHTS.md](docs/MODEL-WEIGHTS.md).

| Project | Licence | Where | Notes |
|---|---|---|---|
| [Handy](https://github.com/cjpais/Handy) by CJ Pais | MIT | `src-tauri/` (legacy 0.2 app) | Inkwell 0.2 was originally based on it. |
| [SQLite](https://sqlite.org) | Public domain | `core/crates/ink-store`, compiled in by `libsqlite3-sys` (`bundled` feature) | The C source ships inside `libsqlite3-sys` (MIT), so `cargo deny` sees only that crate's licence. The version is whichever one the locked `libsqlite3-sys` bundles. |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) and its ggml, by the ggml authors | MIT | `core/crates/ink-engines` (`engine-llama` feature), compiled in by `llama-cpp-sys-2` 0.1.157 from upstream `src/`, `include/`, `ggml/` (ggml 0.24.0) and `tools/mtmd/` | The source ships inside `llama-cpp-sys-2` (MIT OR Apache-2.0), so `cargo deny` sees only that crate's licence. The crate records the llama-cpp-rs commit it was packaged from, `b7bab03da647f07ed373335872643ea7fd0d0af4`; its `llama-cpp-sys-2/llama.cpp` submodule pins llama.cpp at `26394b4e6749a41c3633db040e0987500a5f7013` (looked up on GitHub on 2026-09-26; the package itself does not record it). The rows below are llama.cpp's own vendored code that `mtmd` compiles in. |
| [miniaudio](https://miniaud.io) v0.11.25, by David Reid | Public domain or MIT-0 (scoped exception, accepted 2026-09-26) | llama.cpp `vendor/miniaudio/miniaudio.h`, as above | Audio file decoding in mtmd's helper. |
| [stb_image](https://github.com/nothings/stb) v2.30, by Sean Barrett | MIT (dual-licensed with public domain; taken as MIT) | llama.cpp `vendor/stb/stb_image.h`, as above | Image decoding in mtmd's helper. Compiled in, not called by Inkwell. |
| [xxHash](https://github.com/Cyan4973/xxHash) 0.8.3, by Yann Collet | BSD-2-Clause | llama.cpp `vendor/hash/xxhash/`, as above | Its copyright notice and licence must ship in the app's About screen: BSD-2 requires them with binary distributions. |
| SHA-1 in C, by Steve Reid | Public domain (scoped exception, accepted 2026-09-26) | llama.cpp `vendor/hash/sha1/`, as above | |
| SHA-256, by Igor Pavlov | Public domain (scoped exception, accepted 2026-09-26) | llama.cpp `vendor/hash/sha256/`, as above | |
| rotate-bits, by William Casarin | MIT | llama.cpp `vendor/hash/rotate-bits/`, as above | |

## Rules

- Allowed licences for code: MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib.
- Code adapted from a BSD-licensed project keeps its notice in the file header as well as a row here.
- Native libraries built from source (C and C++ dependencies of an engine) get a row here when their
  build lands, because `cargo deny` does not see them.
