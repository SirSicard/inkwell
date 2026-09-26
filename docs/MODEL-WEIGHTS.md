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
| Dictation final, meeting final | Qwen3-ASR 1.7B, Q8_0 GGUF plus the audio projector | llama.cpp | Apache-2.0 (from the Qwen model; the GGUF repository shows no tag, so confirm before adding the row) | 2.5 GB |
| Live partials (Mac) | Parakeet TDT 0.6B v3, Core ML | FluidAudio | CC-BY-4.0 | registry |
| Live partials (Windows) | Parakeet TDT 0.6B v3, int8 ONNX | sherpa-onnx | CC-BY-4.0 | registry |
| Far-end diarization | Nemotron-3-Diarization, q8_0 | NeMo-Speech.cpp | OpenMDW-1.1 | 0.11 GB |
| Voice activity | Silero VAD | ONNX Runtime | MIT | 2 MB |

Chosen by measurement on public human-labelled sets (AMI meetings, FLEURS English). Whisper,
SenseVoice and MLX builds were measured and are not used.

## Credits

CC-BY-4.0 weights are credited by name, author and licence in the app's About screen and here.
Parakeet TDT v3 is by NVIDIA, under CC-BY-4.0.
