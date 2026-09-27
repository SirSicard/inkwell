#!/usr/bin/env bash
# Times the final pass of a long meeting on the real engines (S2.8's Verify: "finalize time for
# 60 minutes within S0.6's target"). A measurement: run it on an idle Mac (no other builds, on the
# charger), one run at a time. It runs core/crates/ink-pipeline/tests/finalize_time.rs, which loops
# the committed public AMI fixture to the length asked for, writes it as chunks, and finalizes it
# with Silero, Nemotron and Qwen3-ASR; the summary (Foundation Models, in the shell) is not in it.
#
#   INK_BENCH_DIR=<bench> NEMO_SPEECH_DIR=<prefix> mac/scripts/finalize-time.sh
#   INK_FINALIZE_MINUTES=60        the meeting's length (default 60)
#   INK_FINALIZE_TARGET_S=288      the target (default 2 x minutes x 60 / 25: both sides at the
#                                  gate's 25x real time for Qwen3-ASR)
#   INK_FINALIZE_ASSERT=1          fail when over the target (the coordinator's run)
#
# The row lands in $INK_BENCH_DIR/out/s2.8/finalize-time.tsv, never in the repository.
set -euo pipefail

fail() { echo "finalize-time: $*" >&2; exit 1; }
[ -n "${INK_BENCH_DIR:-}" ] || fail "set INK_BENCH_DIR to the bench data (models/ under it)"
[ -n "${NEMO_SPEECH_DIR:-}" ] || fail "set NEMO_SPEECH_DIR to NeMo-Speech.cpp's install prefix"
for model in silero-vad/silero_vad_16k_op15.onnx nemotron-3-diarization qwen3-asr-1.7b-gguf; do
  [ -e "$INK_BENCH_DIR/models/$model" ] || fail "missing $INK_BENCH_DIR/models/$model"
done

root="$(cd "$(dirname "$0")/../.." && pwd)"
echo "finalize-time: $(sysctl -n machdep.cpu.brand_string 2>/dev/null || uname -m), $(sw_vers -productVersion 2>/dev/null || true)"
echo "finalize-time: $(pmset -g batt 2>/dev/null | head -1 || true)"
echo "finalize-time: a measurement is only as good as an idle Mac: close other builds first."

export INK_BENCH_DIR NEMO_SPEECH_DIR
cd "$root/core"
# Built first, so the build's time is never in the measured run.
cargo test -p ink-pipeline --release --features engine-silero,engine-nemo,engine-llama \
  --test finalize_time --no-run ${CARGO_FLAGS:-}
cargo test -p ink-pipeline --release --features engine-silero,engine-nemo,engine-llama \
  --test finalize_time ${CARGO_FLAGS:-} -- --ignored --nocapture 2>&1 | grep -E "finalize time|test result|panicked|FAILED"
echo "finalize-time: rows in $INK_BENCH_DIR/out/s2.8/finalize-time.tsv"
