# Download Inkwell models to the app data directory
# Run this once to set up models for testing

$modelsDir = "$env:APPDATA\com.inkwell.app\models"
New-Item -ItemType Directory -Force -Path $modelsDir | Out-Null

Write-Host "Models directory: $modelsDir" -ForegroundColor Cyan

# --- Silero VAD ---
$vadUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx"
$vadPath = "$modelsDir\silero_vad.onnx"
if (!(Test-Path $vadPath)) {
    Write-Host "Downloading Silero VAD..." -ForegroundColor Yellow
    Invoke-WebRequest -Uri $vadUrl -OutFile $vadPath
    Write-Host "  Done: $vadPath" -ForegroundColor Green
} else {
    Write-Host "Silero VAD already exists" -ForegroundColor Green
}

# --- Moonshine Tiny ---
$moonDir = "$modelsDir\moonshine-tiny"
New-Item -ItemType Directory -Force -Path $moonDir | Out-Null
$moonBase = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models"

# Moonshine Tiny is a V1 model: needs preprocessor, encoder, uncached_decoder, cached_decoder, tokens
$hfBase = "https://huggingface.co/csukuangfj/sherpa-onnx-moonshine-tiny-en-int8/resolve/main"

$filesToDownload = @{
    "$moonDir\preprocess.onnx" = "$hfBase/preprocess.onnx"
    "$moonDir\encode.int8.onnx" = "$hfBase/encode.int8.onnx"
    "$moonDir\uncached_decode.int8.onnx" = "$hfBase/uncached_decode.int8.onnx"
    "$moonDir\cached_decode.int8.onnx" = "$hfBase/cached_decode.int8.onnx"
    "$moonDir\tokens.txt" = "$hfBase/tokens.txt"
}

foreach ($entry in $filesToDownload.GetEnumerator()) {
    if (!(Test-Path $entry.Key)) {
        Write-Host "Downloading $(Split-Path $entry.Key -Leaf)..." -ForegroundColor Yellow
        Invoke-WebRequest -Uri $entry.Value -OutFile $entry.Key
        Write-Host "  Done: $($entry.Key)" -ForegroundColor Green
    } else {
        Write-Host "$(Split-Path $entry.Key -Leaf) already exists" -ForegroundColor Green
    }
}

Write-Host ""
Write-Host "All models ready!" -ForegroundColor Cyan
Write-Host "Restart Inkwell to load them." -ForegroundColor Cyan
