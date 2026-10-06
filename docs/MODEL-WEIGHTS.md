# Model weights

Inkwell never bundles model weights. The app downloads them at runtime, into its data directory,
from a pinned revision with a checked hash. This file is the policy and the list; the engine
registry in `ink-engines` holds each model's pinned revision, sha256 and size.

## Policy

- **Allowed licences:** Apache-2.0, MIT, OpenMDW-1.1, CC-BY-4.0 (credited in About).
- **Not until read and approved:** the NVIDIA Open Model Licence, and Hugging Face's `other`.
- **Never:** non-commercial (NC) weights, OpenRAIL-derived weights, TEN VAD, and preview
  checkpoints.
- **Pin everything:** a registry row names an exact revision, never `/resolve/main`.
- A model whose repository shows no licence tag gets its licence confirmed from the model card before
  its registry row is added.

## Models in 1.0

| Job | Model | Runtime | Licence | Size (approx.) |
|---|---|---|---|---|
| Dictation final, meeting final | Qwen3-ASR 1.7B, Q8_0 GGUF plus the audio projector | llama.cpp | Apache-2.0 (confirmed on the base model `Qwen/Qwen3-ASR-1.7B`'s card; the GGUF repository shows no tag) | 2.5 GB |
| Live partials (Mac) | Parakeet TDT 0.6B v3, Core ML | FluidAudio | CC-BY-4.0 | 0.48 GB |
| Live partials (Windows), and dictation on a PC without a GPU | Parakeet TDT 0.6B v3, int8 ONNX | sherpa-onnx | CC-BY-4.0 | 0.67 GB |
| Far-end diarization | Nemotron-3-Diarization, q8_0 | NeMo-Speech.cpp | OpenMDW-1.1 | 0.11 GB |
| Voice activity | Silero VAD v6.2.3, 16 kHz ONNX | tract (pure Rust) | MIT | 1.3 MB |
| Polish, voice edit, summaries and Ask on this PC (Windows only, optional) | Qwen3-4B-Instruct-2507, Q4_K_M GGUF (unsloth's conversion; Qwen publishes no GGUF of it) | llama.cpp | Apache-2.0 (the repository's card and tag, and the base model's) | 2.5 GB |

The speech models were chosen by measurement on public human-labelled sets (AMI meetings, FLEURS
English). Whisper, SenseVoice and MLX builds were measured and are not used.

The language model was chosen by measurement too (2026-10-06, an RTX 3090 on Vulkan, synthetic
dictations and public AMI transcripts): polish passed its guard on 37 of 43 dictations, at 0.24 s
at the median, and a meeting's harvest found 17 of 20 commitments. Qwen3-1.7B with its thinking
turned off gave the text back unchanged 34 times in 43, so 1.0 offers the one model. On the Mac,
Apple's on-device model does this, and the core lists no language model there. The row's commit,
size and SHA-256 are Hugging Face's listing at that commit, and the bench's downloaded copy has
the same; `ink-engines/tests/language_rows.rs` (ignored) checks a local copy against them.

## Credits

CC-BY-4.0 weights are credited by name, author and licence in the app's About screen and here.
Parakeet TDT v3 is by NVIDIA, under CC-BY-4.0. Silero VAD is by the Silero team, under MIT: its
copyright and licence notice ships with the app and is shown in About. Qwen3-4B-Instruct-2507 is
by the Qwen team (Alibaba Cloud), under Apache-2.0, converted to GGUF by Unsloth; Windows' About
credits it.
