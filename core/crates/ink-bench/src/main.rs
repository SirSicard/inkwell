//! `ink-bench`: replay fixtures through the core and score WER, DER, ERLE and latency.
//!
//! Reads audio only from `$INK_BENCH_DIR`; it never names a private path. The commands land with
//! the engines (S1.4) and the chains (S1.5). Until then it says so and exits non-zero.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("ink-bench: no commands yet (they arrive in S1.4 and S1.5)");
    ExitCode::from(2)
}
