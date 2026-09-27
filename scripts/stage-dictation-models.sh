#!/usr/bin/env bash
# Stages dictation's models (Qwen3-ASR 1.7B and Silero VAD) from $INK_BENCH_DIR into a models
# directory the app reads (INK_MODELS_DIR), as symlinks: nothing is copied or downloaded. Each
# row's id, revision and file names are read from the registry's source, never retyped here, so a
# row that changes is staged as it now is (and one whose files the bench lacks is refused).
#
#   INK_BENCH_DIR=<bench data> scripts/stage-dictation-models.sh <models dir>
#
# Then start the app with INK_MODELS_DIR=<models dir> (mac/DICTATION-CHECKLIST.md).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
: "${INK_BENCH_DIR:?INK_BENCH_DIR must name the bench data directory}"
[ $# -eq 1 ] || { echo "usage: $0 <models dir>" >&2; exit 2; }
models="$1"
fail() { echo "stage-dictation-models: $*" >&2; exit 3; }

# stage <id> <revision> <source dir> <file>...
stage() {
  local id=$1 revision=$2 src=$3
  shift 3
  [ ${#revision} -eq 40 ] || fail "$id: no 40-character revision read"
  local dir="$models/$id/${revision:0:12}"
  mkdir -p "$dir"
  for f in "$@"; do
    [ -f "$src/$f" ] || fail "missing $src/$f"
    ln -sf "$src/$f" "$dir/$f"
  done
  printf '%s' "$revision" >"$dir/.revision"
  echo "staged $id ($*) in $dir"
}

registry="$root/core/crates/ink-engines/src/registry.rs"
row="$(sed -n '/^fn qwen3_asr_1_7b_q8() -> EngineRow {/,/^}/p' "$registry")"
revision="$(sed -nE 's/.*const REVISION: &str = "([0-9a-f]{40})";.*/\1/p' <<<"$row")"
id="$(sed -nE 's/^ *id: "([^"]+)"\.into\(\),.*/\1/p' <<<"$row")"
files="$(grep -oE '"[A-Za-z0-9._-]+\.gguf"' <<<"$row" | tr -d '"' | sort -u | tr '\n' ' ')"
[ -n "$id" ] && [ -n "$files" ] || fail "could not read the dictation model's row from $registry"
# shellcheck disable=SC2086 # the file names are words
stage "$id" "$revision" "$INK_BENCH_DIR/models/qwen3-asr-1.7b-gguf" $files

rows="$root/core/crates/ink-engines/src/rows.rs"
vad_id="$(sed -nE 's/^pub const SILERO_VAD_ID: &str = "([^"]+)";/\1/p' "$rows")"
row="$(sed -n '/^pub fn silero_vad() -> EngineRow {/,/^}/p' "$rows")"
revision="$(sed -nE 's/.*const REVISION: &str = "([0-9a-f]{40})";.*/\1/p' <<<"$row")"
file="$(sed -nE 's/.*const FILE: &str = "([^"]+)";.*/\1/p' <<<"$row")"
[ -n "$vad_id" ] && [ -n "$file" ] || fail "could not read the VAD's row from $rows"
stage "$vad_id" "$revision" "$INK_BENCH_DIR/models/silero-vad" "$file"
