#!/usr/bin/env bash
# The shell budget (invariant I7), measured on the built Mac app with the dictation model warm:
#
#   idle    window open, nothing live, SAMPLES s:  0 frames drawn and < 0.1 % of one core (mean)
#   live    the ink at 60 fps with no audio, SAMPLES s:  <= 6 % p50 and <= 10 % p95 of one core
#   memory  footprint at the end of the idle window:  <= 150 MB + the dictation model's size
#
# Runs are timed one after another, never alongside another benchmark: run this only on a quiet
# Mac with its screen unlocked (a hidden window draws nothing and would fake a low number).
#
#   scripts/idle-budget.sh [all|idle|live]         (default all: about 2 x (SAMPLES + warm-up))
#
# Build first, with the llama.cpp engine so the dictation model can be warmed:
#   INK_SIGN_IDENTITY=... INK_CORE_FEATURES=engine-llama mac/scripts/build-mac.sh
#
# Environment:
#   INK_APP          the app (default mac/build/Inkwell.app)
#   INK_MODELS_DIR   a models directory with the dictation model installed, or
#   INK_BENCH_DIR    stage one from $INK_BENCH_DIR/models/qwen3-asr-1.7b-gguf (symlinks; nothing
#                    is copied or downloaded); the output also goes under $INK_BENCH_DIR/out
#   SAMPLES=120      seconds sampled per run (I7 is "over 2 min")
#   SETTLE=15        seconds between the model being warm and the start of sampling
#   WARM_TIMEOUT=300 seconds to wait for the model to warm
#   BUSY_MAX=15      refuse a run while the whole machine is busier than this (%)
#   FORCE=1          measure anyway when a preflight refuses (the summary says so)
#   OUT=<dir>        output directory
#
# Exit: 0 every phase asked for passed; 1 a budget was exceeded; 2 a phase could not be measured
# (void: slept, hidden window, or nothing drew); 3 refused before measuring.
#
# The app runs with its own library in the output directory (INK_DATA_DIR), never the user's.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
what="${1:-all}"
case "$what" in all | idle | live) ;; *) echo "usage: $0 [all|idle|live]" >&2; exit 2 ;; esac

app="${INK_APP:-$root/mac/build/Inkwell.app}"
exe="$app/Contents/MacOS/Inkwell"
SAMPLES="${SAMPLES:-120}"
SETTLE="${SETTLE:-15}"
WARM_TIMEOUT="${WARM_TIMEOUT:-300}"
BUSY_MAX="${BUSY_MAX:-15}"
stamp="$(date +%Y%m%d-%H%M%S)"
if [ -n "${OUT:-}" ]; then
  out="$OUT"
elif [ -n "${INK_BENCH_DIR:-}" ]; then
  out="$INK_BENCH_DIR/out/idle-budget/$stamp"
else
  out="${TMPDIR:-/tmp}/inkwell-idle-budget/$stamp"
fi
summary="$out/SUMMARY.txt"
mkdir -p "$out"

say() { echo "$*" | tee -a "$summary"; }
refuse() {
  if [ "${FORCE:-0}" = 1 ]; then
    say "FORCE=1, measuring anyway ($*): treat the numbers as contaminated"
  else
    say "REFUSED: $*  (FORCE=1 overrides)"
    exit 3
  fi
}

[ -x "$exe" ] || { echo "no app at $app: build it first (see the header)" >&2; exit 3; }

# --- the model ---------------------------------------------------------------------------------
# Staging reads the row's id, revision and file names from the registry source, so they are the
# core's own, never retyped here. If they drift, the app reports the model as not installed and
# the run stops at the warm-up.
if [ -n "${INK_MODELS_DIR:-}" ]; then
  models="$INK_MODELS_DIR"
elif [ -n "${INK_BENCH_DIR:-}" ]; then
  registry="$root/core/crates/ink-engines/src/registry.rs"
  row="$(sed -n '/^fn qwen3_asr_1_7b_q8() -> EngineRow {/,/^}/p' "$registry")"
  revision="$(sed -nE 's/.*const REVISION: &str = "([0-9a-f]{40})";.*/\1/p' <<<"$row")"
  id="$(sed -nE 's/^ *id: "([^"]+)"\.into\(\),.*/\1/p' <<<"$row")"
  files="$(grep -oE '"[A-Za-z0-9._-]+\.gguf"' <<<"$row" | tr -d '"' | sort -u)"
  [ -n "$revision" ] && [ -n "$id" ] && [ -n "$files" ] \
    || { echo "could not read the dictation model's row from $registry" >&2; exit 3; }
  models="$out/models"
  dir="$models/$id/${revision:0:12}"
  mkdir -p "$dir"
  for f in $files; do
    src="$INK_BENCH_DIR/models/qwen3-asr-1.7b-gguf/$f"
    [ -f "$src" ] || { echo "missing $src" >&2; exit 3; }
    ln -sf "$src" "$dir/$f"
  done
  printf '%s' "$revision" >"$dir/.revision"
else
  echo "set INK_MODELS_DIR (installed models) or INK_BENCH_DIR (to stage them)" >&2
  exit 3
fi

# --- preflight ---------------------------------------------------------------------------------
say "idle-budget $(date '+%Y-%m-%d %H:%M:%S') what=$what samples=$SAMPLES settle=$SETTLE host=$(sysctl -n machdep.cpu.brand_string) macOS=$(sw_vers -productVersion)"
say "app: $app $(defaults read "$app/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo '?')"
say "output: $out"

# Power: a low battery or a small adapter throttles the machine and voids timings (it did once).
batt="$(pmset -g batt)"
adapter="$(pmset -g adapter)"
source_line="$(head -1 <<<"$batt" | sed -E "s/.*'(.*)'.*/\1/")"
percent="$(grep -oE '[0-9]+%' <<<"$batt" | head -1 | tr -d %)"
watts="$(sed -nE 's/^ *Wattage = ([0-9]+)W.*/\1/p' <<<"$adapter" | head -1)"
powermode="$(pmset -g | sed -nE 's/^ *powermode +([0-9]+).*/\1/p' | head -1)"
say "power: ${source_line:-?}, battery ${percent:-?} %, adapter ${watts:-none} W, powermode ${powermode:-?}"
[ "${powermode:-0}" = 1 ] && refuse "Low Power Mode is on"
if [ "$source_line" = "Battery Power" ] && [ "${percent:-100}" -lt 30 ]; then
  refuse "on battery at ${percent} %"
fi
if [ "$source_line" = "AC Power" ] && [ -n "$watts" ] && [ "$watts" -lt 60 ]; then
  say "WARNING: a ${watts} W adapter can limit this Mac under load"
fi

locked="$(ioreg -n Root -d1 -a | plutil -extract IOConsoleUsers.0.CGSSessionScreenIsLocked raw - 2>/dev/null || echo false)"
[ "$locked" = true ] && refuse "the screen is locked: the window cannot be visible"

if pgrep -f "Inkwell.app/Contents/MacOS/Inkwell\$" >/dev/null 2>&1; then
  refuse "another Inkwell is running: quit it (the measured copy would find it and exit)"
fi
pgrep -f '/Applications/Inkwell.app/Contents/MacOS/app$' >/dev/null 2>&1 \
  && say "note: Inkwell 0.2 is running; it is not measured, but it shares the machine"

caffeinate -d -i -m -w $$ &

# The whole machine's busy share, from the second top sample.
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

# --- measuring ---------------------------------------------------------------------------------
# Cumulative CPU seconds of a pid, from ps's [[dd-]hh:]mm:ss.ss.
cputime() {
  ps -o time= -p "$1" | awk '{
    n = split($1, p, ":"); s = 0
    for (i = 1; i <= n; i++) { v = p[i]; if (i == 1 && index(v, "-")) { split(v, d, "-"); s = d[1] * 24; v = d[2] } s = s * 60 + v }
    print s }'
}

# Mean / p50 / p95 / max of the pid's %CPU in a top log. The first sample is dropped: top's first
# reading of a process is not an interval.
top_stats() {
  awk -v pid="$2" '$1 == pid { print $2 }' "$1" | tail -n +2 | sort -n | awk '
    { a[NR] = $1; s += $1 }
    END {
      if (NR == 0) { print "samples=0"; exit }
      printf "samples=%d mean=%.2f p50=%.1f p95=%.1f max=%.1f\n", NR, s / NR, a[int((NR - 1) * 0.50) + 1], a[int((NR - 1) * 0.95) + 1], a[NR]
    }'
}

# Footprint in MiB: `footprint` prints "Footprint: 111 MB" (or KB, GB).
footprint_mib() {
  footprint "$1" >"$2" 2>&1 || true
  awk '/Footprint:/ { for (i = 1; i <= NF; i++) if ($i == "Footprint:") { v = $(i + 1); u = $(i + 2)
         if (u == "KB") v /= 1024; else if (u == "GB") v *= 1024; else if (u == "B") v /= 1048576
         printf "%.1f\n", v; exit } }' "$2"
}

# Asks the app for a frames mark and prints it ("frames=N visible=true").
frames_mark() {
  local pid=$1 log=$2 before i
  before="$(grep -c '^mark frames=' "$log" || true)"
  kill -USR1 "$pid"
  for i in $(seq 1 50); do
    [ "$(grep -c '^mark frames=' "$log" || true)" -gt "$before" ] && break
    sleep 0.1
  done
  grep '^mark frames=' "$log" | tail -1 | sed 's/^mark //' || true
}

# Sleep, DarkWake or Wake entries in the power log between two "YYYY-MM-DD HH:MM:SS" times.
slept_between() {
  pmset -g log | awk -v a="$1" -v b="$2" '
    /^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9:]{8} / {
      t = $1 " " $2
      if (t >= a && t <= b && ($4 == "Sleep" || $4 == "DarkWake" || ($4 == "Wake" && $5 != "Requests"))) print
    }'
}

verdicts=""
failed=0
unmeasured=0
verdict() { # phase status detail
  verdicts="$verdicts
  $1: $2  $3"
  case "$2" in
    FAIL) failed=1 ;;
    VOID | "NOT MEASURED") unmeasured=1 ;;
  esac
  return 0
}

model_mib=""

# One run: launch in `state`, wait for the model, sample, read the marks, quit cleanly.
run() {
  local state=$1 log="$out/$state.log" pid i m0 m1 f0 f1 v0 v1 c0 c1 t0 t1 w0 w1 stats p50 p95 mean fps fp rss slept status marks_ok
  preflight "$state"
  INK_MEASURE="$state" INK_DATA_DIR="$out/data-$state" INK_MODELS_DIR="$models" "$exe" >"$log" 2>&1 &
  pid=$!
  say "[$state] pid=$pid; waiting for the dictation model (up to ${WARM_TIMEOUT} s)"
  for i in $(seq 1 $((WARM_TIMEOUT * 5))); do
    grep -q '^mark event=model.warmed job=dictation_final' "$log" && break
    if grep -q '^mark event=model.warm_failed job=dictation_final' "$log"; then
      say "[$state] the model did not warm: $(grep '^mark event=model.warm_failed' "$log" | tail -1)"
      kill -TERM "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
      exit 3
    fi
    kill -0 "$pid" 2>/dev/null || { say "[$state] the app exited early:"; cat "$log"; exit 3; }
    sleep 0.2
  done
  grep -q '^mark event=model.warmed job=dictation_final' "$log" \
    || { say "[$state] no warm model after ${WARM_TIMEOUT} s"; kill -TERM "$pid" 2>/dev/null || true; exit 3; }
  say "[$state] $(grep '^mark event=model.warmed' "$log" | tail -1 | sed 's/^mark //')"
  if [ -z "$model_mib" ]; then
    local id row_dir
    id="$(grep '^mark event=model.warmed job=dictation_final' "$log" | tail -1 | sed -E 's/.* id=([^ ]+).*/\1/')"
    row_dir="$(find "$models/$id" -mindepth 1 -maxdepth 1 -type d | head -1)"
    model_mib="$(find "$row_dir" -mindepth 1 -maxdepth 1 ! -name '.*' -exec stat -L -f %z {} + | awk '{ s += $1 } END { printf "%.1f\n", s / 1048576 }')"
    say "model: $id, ${model_mib} MiB on disk"
  fi
  sleep "$SETTLE"

  m0="$(frames_mark "$pid" "$log")"
  c0="$(cputime "$pid")"; t0="$(date +%s)"; w0="$(date '+%Y-%m-%d %H:%M:%S')"
  say "[$state] sampling ${SAMPLES} s from $w0"
  top -l "$SAMPLES" -s 1 -stats pid,cpu -pid "$pid" >"$out/top-$state.txt"
  c1="$(cputime "$pid")"; t1="$(date +%s)"; w1="$(date '+%Y-%m-%d %H:%M:%S')"
  m1="$(frames_mark "$pid" "$log")"
  fp="$(footprint_mib "$pid" "$out/footprint-$state.txt")"
  rss="$(ps -o rss= -p "$pid" | awk '{ printf "%.1f", $1 / 1024 }')"

  stats="$(top_stats "$out/top-$state.txt" "$pid")"
  mean="$(awk -v a="$c0" -v b="$c1" -v w="$((t1 - t0))" 'BEGIN { printf "%.3f", (b - a) / w * 100 }')"
  f0="$(sed -nE 's/^frames=([0-9]+) .*/\1/p' <<<"$m0")"; f1="$(sed -nE 's/^frames=([0-9]+) .*/\1/p' <<<"$m1")"
  v0="$(sed -nE 's/.*visible=([a-z]+).*/\1/p' <<<"$m0")"; v1="$(sed -nE 's/.*visible=([a-z]+).*/\1/p' <<<"$m1")"
  marks_ok=1
  if [ -z "$f0" ] || [ -z "$f1" ]; then
    marks_ok=0
    f0=0
    f1=0
  fi
  fps="$(awk -v a="${f0:-0}" -v b="${f1:-0}" -v w="$((t1 - t0))" 'BEGIN { printf "%.1f", (b - a) / w }')"
  say "[$state] top: $stats"
  say "[$state] cpu-time: mean ${mean} % of one core over $((t1 - t0)) s"
  say "[$state] frames: $((f1 - f0)) in the window (${fps}/s); visible at start=$v0 end=$v1"
  say "[$state] footprint: ${fp:-?} MiB (resident ${rss:-?} MiB)"

  kill -TERM "$pid"
  status=0
  for i in $(seq 1 600); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
  if kill -0 "$pid" 2>/dev/null; then
    say "[$state] the app did not quit within 60 s"
    kill -KILL "$pid"
    status=1
  fi
  wait "$pid" 2>/dev/null || status=$?
  # A crash on the way out is what a model still loaded at exit looks like (ggml's Metal backend).
  [ "$status" = 0 ] && say "[$state] quit cleanly" || say "[$state] WARNING: exit status $status on quit"

  slept="$(slept_between "$w0" "$w1")"
  if [ -n "$slept" ]; then
    say "[$state] the Mac slept or woke inside the window:"
    say "$slept"
    verdict "$state" VOID "slept inside the window"
    return 0
  fi
  if [ "$marks_ok" = 0 ]; then
    verdict "$state" VOID "the app did not answer the frame marks"
    return 0
  fi
  if [ "$v0" != true ] || [ "$v1" != true ]; then
    verdict "$state" VOID "the window was not visible, so nothing had to draw"
    return 0
  fi

  p50="$(sed -E 's/.*p50=([0-9.]+).*/\1/' <<<"$stats")"; p95="$(sed -E 's/.*p95=([0-9.]+).*/\1/' <<<"$stats")"
  case "$state" in
    idle)
      if [ "$((f1 - f0))" -eq 0 ] && awk -v m="$mean" 'BEGIN { exit !(m < 0.1) }'; then
        verdict idle PASS "0 frames, ${mean} % mean (< 0.1 %)"
      else
        verdict idle FAIL "$((f1 - f0)) frames, ${mean} % mean (budget: 0 frames, < 0.1 %)"
      fi
      if [ -z "$fp" ]; then
        verdict memory VOID "footprint could not be read (see $out/footprint-idle.txt)"
      elif awk -v f="$fp" -v m="$model_mib" 'BEGIN { exit !(f <= 150 + m) }'; then
        verdict memory PASS "${fp} MiB <= 150 + ${model_mib} MiB"
      else
        verdict memory FAIL "${fp} MiB > 150 + ${model_mib} MiB"
      fi
      ;;
    live)
      if awk -v f="$fps" 'BEGIN { exit !(f < 55) }'; then
        verdict live "NOT MEASURED" "${fps} frames/s: nothing drew at 60 fps (no ink renderer in this build?)"
      elif awk -v a="$p50" -v b="$p95" 'BEGIN { exit !(a <= 6 && b <= 10) }'; then
        verdict live PASS "p50 ${p50} %, p95 ${p95} % at ${fps} fps"
      else
        verdict live FAIL "p50 ${p50} %, p95 ${p95} % at ${fps} fps (budget: <= 6 %, <= 10 %)"
      fi
      ;;
  esac
}

case "$what" in
  all) run idle; run live ;;
  idle) run idle ;;
  live) run live ;;
esac

say ""
say "I7 verdict:$verdicts"
say "done: $summary"
if [ "$failed" = 1 ]; then
  exit 1
elif [ "$unmeasured" = 1 ]; then
  exit 2
fi
