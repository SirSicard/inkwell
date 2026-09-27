#!/usr/bin/env bash
# Dictation latency in replay (plan S2.7): from the simulated key-up to the insertion request,
# through the real dictation chain with Qwen3-ASR 1.7B and Silero, on 5 s FLEURS English clips.
#
#   warm     20 takes back to back:                   target p50 <= 350 ms, p95 <= 700 ms
#   idle     the first take after IDLE s of quiet,    once with the key-down warm-up and once
#            RUNS_IDLE times each                     without, to show what the warm-up saves
#
# Runs one after another, never alongside another benchmark or a build: run it only on a quiet
# Mac (it refuses while the machine is busier than BUSY_MAX %). The shipped configuration is
# measured: ggml's Metal residency off (GGML_METAL_NO_RESIDENCY=1), as the Mac app runs it.
#
#   scripts/dictation-latency.sh [all|warm|idle]      (default all: about 2 x RUNS_IDLE x IDLE s)
#
# Environment:
#   INK_BENCH_DIR   required: fleurs/en_us/dev/*.wav, models/qwen3-asr-1.7b-gguf/,
#                   models/silero-vad/silero_vad_16k_op15.onnx; rows go to out/s2.7/
#   RUNS=20         takes in the warm phase
#   RUNS_IDLE=5     takes per idle condition
#   IDLE=240        seconds of quiet before each idle take (the residency measurement's 240 s)
#   SECONDS_EACH=5  utterance length
#   BUSY_MAX=15     refuse while the machine is busier than this (%)
#   FORCE=1         measure anyway when the preflight refuses (the summary says so)
#   SMOKE=1         one short run of each phase, to prove the script works (not a measurement);
#                   its idle is 35 s, just past the 30 s after which a take's start warms the engine
#
# Exit: 0 every phase ran (the numbers say whether the targets were met); 2 a phase failed;
# 3 refused before measuring.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
what="${1:-all}"
case "$what" in all | warm | idle) ;; *) echo "usage: $0 [all|warm|idle]" >&2; exit 2 ;; esac
: "${INK_BENCH_DIR:?INK_BENCH_DIR must name the bench data directory}"

RUNS="${RUNS:-20}"
RUNS_IDLE="${RUNS_IDLE:-5}"
IDLE="${IDLE:-240}"
SECONDS_EACH="${SECONDS_EACH:-5}"
BUSY_MAX="${BUSY_MAX:-15}"
if [ "${SMOKE:-0}" = 1 ]; then
  RUNS=2 RUNS_IDLE=1 IDLE=35
fi
stamp="$(date +%Y%m%d-%H%M%S)"
out="$INK_BENCH_DIR/out/s2.7/latency-$stamp"
mkdir -p "$out"
say() { echo "dictation-latency: $*" | tee -a "$out/summary.txt"; }
refuse() {
  if [ "${FORCE:-0}" = 1 ]; then
    say "PREFLIGHT REFUSED, measured anyway (FORCE=1): $*"
  else
    say "refused: $*"
    exit 3
  fi
}

# As the Mac app runs (MetalResidency.swift): residency off.
export GGML_METAL_NO_RESIDENCY=1

say "building ink-bench with Qwen3-ASR and Silero (release)"
(cd "$root/core" && cargo build --release --locked -p ink-bench --features engine-llama,engine-silero) \
  >"$out/build.log" 2>&1 || { say "the build failed: $out/build.log"; exit 2; }
bench="$root/core/target/release/ink-bench"

# The whole machine's busy share, from the second top sample (as scripts/idle-budget.sh).
preflight() {
  local label=$1 busy
  top -l 2 -s 1 -n 10 -o cpu -stats pid,command,cpu >"$out/preflight-$label.txt"
  busy="$(grep 'CPU usage' "$out/preflight-$label.txt" | tail -1 \
    | awk '{ for (i = 1; i <= NF; i++) if ($i == "idle") { v = $(i - 1); sub("%", "", v); print 100 - v } }')"
  say "[$label] preflight: machine ${busy} % busy (limit ${BUSY_MAX} %)"
  if awk -v b="$busy" -v m="$BUSY_MAX" 'BEGIN { exit !(b > m) }'; then
    refuse "the machine is busy (top processes in $out/preflight-$label.txt)"
  fi
}

caffeinate -i -m -w $$ &

phase() {
  local label=$1
  shift
  preflight "$label"
  say "[$label] ink-bench latency $*"
  if ! "$bench" latency --seconds "$SECONDS_EACH" --out "$out/$label.tsv" "$@" \
    >"$out/$label.txt" 2>"$out/$label.log"; then
    say "[$label] FAILED: $out/$label.log"
    return 2
  fi
  sed 's/^/  /' "$out/$label.txt" | tee -a "$out/summary.txt"
}

status=0
if [ "$what" = all ] || [ "$what" = warm ]; then
  phase warm --runs "$RUNS" || status=2
fi
if [ "$what" = all ] || [ "$what" = idle ]; then
  phase idle-warmup-on --runs "$RUNS_IDLE" --idle "$IDLE" --warmup on || status=2
  phase idle-warmup-off --runs "$RUNS_IDLE" --idle "$IDLE" --warmup off || status=2
fi
[ "${SMOKE:-0}" = 1 ] && say "SMOKE run: not a measurement"
say "rows and logs: $out"
exit "$status"
