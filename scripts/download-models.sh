#!/bin/sh
# Download Inkwell models to the app data directory (macOS / Linux).
# POSIX equivalent of download-models.ps1 — same models, same destination.
# Run this once to set up models for testing.

set -eu

APP_ID="com.inkwell.app"

case "$(uname -s)" in
    Darwin) MODELS_DIR="$HOME/Library/Application Support/$APP_ID/models" ;;
    *)      MODELS_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/$APP_ID/models" ;;
esac

mkdir -p "$MODELS_DIR"
echo "Models directory: $MODELS_DIR"

# Download to a temp file first so an interrupted run doesn't leave a truncated
# model behind that the existence check would then treat as complete.
fetch() {
    dest="$1"
    url="$2"
    name="$(basename "$dest")"

    if [ -f "$dest" ]; then
        echo "$name already exists"
        return 0
    fi

    echo "Downloading $name..."
    tmp="$dest.part"
    # -L / redirects matter: GitHub release assets 302 to a signed CDN URL.
    if command -v curl >/dev/null 2>&1; then
        curl -fL --progress-bar -o "$tmp" "$url" || { rm -f "$tmp"; echo "Error: failed to download $name" >&2; exit 1; }
    elif command -v wget >/dev/null 2>&1; then
        wget -q --show-progress -O "$tmp" "$url" || { rm -f "$tmp"; echo "Error: failed to download $name" >&2; exit 1; }
    else
        echo "Error: neither curl nor wget is available" >&2
        exit 1
    fi
    mv "$tmp" "$dest"
    echo "  Done: $dest"
}

# --- Silero VAD ---
# The app also fetches this on first run (setup::ensure_vad_model); same URL.
# This script is the manual fallback when that fetch is blocked or fails.
fetch "$MODELS_DIR/silero_vad.onnx" \
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx"

echo ""
echo "All models ready!"
echo "Restart Inkwell to load them."
