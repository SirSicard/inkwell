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

# Speech models are downloaded from inside the app (Settings > Models), which is
# the only path that matches what the app knows how to load. This script exists
# for the VAD model, which the app fetches on first run but cannot prompt for.

Write-Host ""
Write-Host "All models ready!" -ForegroundColor Cyan
Write-Host "Restart Inkwell to load them." -ForegroundColor Cyan
