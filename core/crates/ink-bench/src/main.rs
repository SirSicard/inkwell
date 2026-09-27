//! `ink-bench`: replay fixtures through the core and measure it.
//!
//! Reads audio only from where it is told (`$INK_BENCH_DIR` by default); it never names a private
//! path. Commands:
//!
//! ```text
//! ink-bench latency [--clips DIR] [--runs N] [--seconds S] [--idle SECS] [--warmup on|off]
//!                   [--pace realtime|fast] [--engine mock|qwen] [--vad silero|none]
//!                   [--mock-delay-ms MS] [--out FILE]
//! ```
//!
//! See [`ink_bench::latency`] for what `latency` measures.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("latency") => match ink_bench::latency::Config::from_args(&args[1..])
            .and_then(|config| ink_bench::latency::run(&config))
        {
            Ok(report) => {
                println!("{}", report.summary());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("ink-bench latency: {e}");
                ExitCode::from(2)
            }
        },
        _ => {
            eprintln!("usage: ink-bench latency [options]   (see the crate docs)");
            ExitCode::from(2)
        }
    }
}
