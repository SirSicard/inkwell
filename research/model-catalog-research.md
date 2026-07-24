# Inkwell Model Catalog Research
*2026-03-29 | Nix*

## Current State
Inkwell ships with 11 models (sherpa-onnx v1.12.33, static builds, CPU-only):

| Model | Type | Size (int8) | Languages | Notes |
|-------|------|-------------|-----------|-------|
| Moonshine Tiny | moonshine | 70 MB | English | Bundled fallback |
| Parakeet V3 | nemo_transducer | 670 MB | 25 European | **Default/recommended** |
| Whisper Tiny | whisper | 98 MB | 99 languages | |
| Whisper Base | whisper | 135 MB | 99 languages | |
| Whisper Small | whisper | 375 MB | 99 languages | |
| Whisper Medium | whisper | 1.0 GB | 99 languages | |
| Whisper Large V3 | whisper | 1.5 GB | 99 languages | |
| Whisper Turbo | whisper | 800 MB | 99 languages | |
| Whisper Distil Small EN | whisper | 180 MB | English | |
| Whisper Distil Medium EN | whisper | 460 MB | English | |
| SenseVoice | sense_voice | 160 MB | zh/en/ja/ko/yue | |

## New Models Available (sherpa-onnx compatible, free, downloadable)

### HIGH VALUE - Should Add

#### 1. Moonshine Base (English)
- **Repo:** csukuangfj/sherpa-onnx-moonshine-base-en-int8
- **Type:** moonshine (V1 layout: preprocess + encode + cached_decode + uncached_decode)
- **Size:** ~288 MB (encode 50MB, cached_decode 100MB, uncached_decode 122MB, preprocess 14MB)
- **Languages:** English only
- **Why:** Direct upgrade path from Moonshine Tiny. Same engine constructor works (V1 layout auto-detected). Better accuracy, still fast.
- **Effort:** LOW. Just add to download_model registry + MODEL_CATALOG in frontend.

#### 2. Parakeet V2 (English only)
- **Repo:** csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8
- **Type:** nemo_transducer (same as V3: encoder + decoder + joiner)
- **Size:** ~670 MB (similar to V3)
- **Languages:** English only (V3 has 25 languages, V2 is English-specialized)
- **Why:** For users who only need English, V2 may be more accurate for English-only use. Same engine constructor.
- **Effort:** LOW. Same transducer loading code.

#### 3. Canary 180M Flash (EN/ES/DE/FR)
- **Repo:** csukuangfj/sherpa-onnx-nemo-canary-180m-flash-en-es-de-fr-int8
- **Type:** canary (encoder + decoder)
- **Size:** ~207 MB
- **Languages:** English, Spanish, German, French
- **Why:** Small, fast, 4 major European languages. Perfect for multilingual users who don't need 25 languages.
- **Effort:** MEDIUM. Need to add Canary engine constructor to engine.rs. Uses `config.model_config.canary.encoder/decoder` fields. Check if sherpa-onnx v1.12 Rust API exposes CanaryModelConfig.

### MEDIUM VALUE - Consider Adding

#### 4. Omnilingual ASR 300M (1600+ languages)
- **Repo:** csukuangfj/sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-2025-11-12
- **Type:** CTC (single model.onnx file)
- **Size:** ~1.3 GB
- **Languages:** 1600+ (Meta's Omnilingual ASR)
- **Why:** Universal language coverage. Great for niche languages no other model supports.
- **Effort:** MEDIUM. Need to verify which CTC config field this uses (zipformer_ctc? omnilingual? nemo_ctc?). May need sherpa-onnx upgrade.

#### 5. FunASR Nano / SenseVoice Nano
- **Repo:** csukuangfj/sherpa-onnx-sense-voice-funasr-nano-2025-12-17
- **Type:** sense_voice (single model.onnx)
- **Size:** ~1.03 GB (fp32 only, no int8 available in this repo)
- **Languages:** 31 languages, optimized for far-field/noisy, Chinese dialects
- **Why:** Great for noisy environments, meeting rooms. 31 language free switching.
- **Effort:** LOW if sense_voice config works. But 1GB fp32 is large. Check if int8 variant exists elsewhere.

#### 6. FireRed ASR Large (Chinese + English)
- **Repo:** csukuangfj/sherpa-onnx-fire-red-asr-large-zh_en-2025-02-16
- **Type:** fire_red_asr (encoder + decoder)
- **Size:** ~1.74 GB
- **Languages:** Chinese + English (mandarin, cantonese, 20+ Chinese dialects)
- **Why:** Best Chinese ASR available. Far better than Whisper for Chinese.
- **Effort:** MEDIUM. Need fire_red_asr engine constructor. Check Rust API support.

### NICHE - Only if Requested

#### 7. GigaSpeech Zipformer (English)
- **Repo:** k2-fsa/sherpa-onnx-zipformer-gigaspeech-2023-12-12
- **Type:** transducer (encoder + decoder + joiner)
- **Size:** Unknown (repo flagged with suspicious files)
- **Why:** Trained on 10K hours of YouTube/podcast English. Good for conversational speech.
- **Skip reason:** Suspicious file flag on HuggingFace.

#### 8. MedASR CTC (English Medical)
- **Repo:** csukuangfj/sherpa-onnx-medasr-ctc-en-2025-12-25
- **Type:** CTC
- **Size:** ~427 MB
- **Languages:** English (medical/radiology)
- **Why:** Specialized for medical terminology. Niche but unique.

#### 9. Language-Specific Zipformers
Multiple single-language models available:
- Japanese (ReazonSpeech)
- Korean
- Vietnamese
- Thai
- Cantonese
- Russian (GigaAM, multiple versions)
- Chinese (WenetSpeech, multiple sizes)

These use the transducer config (encoder + decoder + joiner). Same loading code as Parakeet but model_type varies.

## Integration Complexity

### Easy (just add to registry, existing engine constructors work):
- Moonshine Base ✅
- Parakeet V2 ✅
- Any zipformer transducer models (same config as existing transducer)

### Medium (need new engine constructor, check Rust API):
- Canary (encoder + decoder, different config path)
- FireRed ASR (encoder + decoder, different config path)
- CTC models (single model file, various config paths)

### Harder (may need sherpa-onnx version bump):
- Omnilingual ASR (newer model type, may need v1.12.x+)
- FunASR Nano (newer model type)
- Dolphin (newer model type, no models found yet)

## Recommendation

**Phase 1 (easy wins, no code changes needed):**
1. Moonshine Base - fills the gap between Tiny (70MB) and Whisper Small (375MB) for English
2. Parakeet V2 - English-specialized alternative to V3's 25-language support

**Phase 2 (new engine constructors):**
3. Canary 180M Flash - small, fast, 4-language. Great UX story ("lightweight multilingual")

**Phase 3 (explore):**
4. Omnilingual 300M - wild card, 1600 languages
5. FireRed ASR - if Chinese market matters
