# Streaming spike: can we show text while the user is still speaking?

**Date:** 2026-07-31
**Verdict:** Viable, and cheaper than expected, but only as *feedback*. The final
text must still come from the offline model. Recommend a two-pass design; do not
ship streaming as the transcription path.

> **Built 2026-08-26, as "Live Preview".** Kept as written, because it is the
> record of the decision. Two corrections it earned on the way into the app:
>
> - **The download is 73 MB, not 296.** The sizes in the table below are
>   HuggingFace *directory* sizes, and those repos carry both precisions plus
>   two left-context variants. The four files a shipped int8 build actually
>   fetches are a tenth of that. This mattered: the download was named here as
>   "most of the objection".
> - **Endpointing must be off, not on.** The harness set `enable_endpoint =
>   true` because it was decoding a whole file. In the app one held hotkey is
>   one utterance, and endpointing resets the stream when it fires, so leaving
>   it on would blank the line whenever the user paused for breath.
>
> Also unaccounted for here: "feed the overlay" assumed the overlay could hold
> a sentence, and it is a 97px square. It now widens to 560px when the feature
> is on. See `src-tauri/src/streaming.rs` and `examples/streaming_check.rs`.

Reproduce with `cargo run --release --example streaming_spike -- <model-dir> <wav>`.

## What was measured

One sentence of synthetic speech, 8.2 seconds, fed through sherpa-onnx's
`OnlineRecognizer` in 100 ms blocks on an M-series Mac, 2 threads, int8.

| Model | Size | Decode time | Real-time factor | First partial |
|---|---|---|---|---|
| Streaming Zipformer EN 20M | 122 MB | 104 ms | **79x** | 13 ms |
| Streaming Zipformer EN | 296 MB | 203 ms | **40x** | 18 ms |
| Parakeet V3 (offline, current default) | 670 MB | 555 ms | 15x | n/a, no partials |

## Findings

**1. Speed is a non-issue.** This was the risk going in and it evaporated.
The larger streaming model decodes 40x faster than real time on two threads,
leaving the other cores free for the offline pass. Partials land about 4 ms
after each 300 ms of audio, which is imperceptible. Running both models
concurrently is affordable.

**2. sherpa-onnx 1.12 already supports it.** `OnlineRecognizer` is in the crate
we ship, with the same transducer shape as Parakeet and support for hotwords and
endpointing. No dependency upgrade, unlike the Qwen3 candidate.

**3. Streaming output is not shippable as final text.** Two independent defects,
both visible in the transcripts:

- **Casing and punctuation are absent.** Streaming models emit
  `I'M TESTING WHETHER STREAMING TRANSCRIPTION CAN SHOW TEXT`. The offline model
  emits `I'm testing whether streaming transcription can show text, ...`.
- **The tail is lost.** Both streaming models ended at `...FAST ENOUGH TO BE`,
  dropping the final word, even after `input_finished()` and draining the
  decoder. The offline model on the same audio produced `...fast enough to be
  useful.` A streaming decoder needs right-context it does not have at the end
  of an utterance; the offline pass sees the whole waveform.

**4. Model size buys accuracy at the start.** The 20M model mangled the opening
into `'STING WHETHER`, losing the first three words. The 296 MB model got
`I'M TESTING WHETHER`. If this ships, it ships the larger one.

## Recommendation

**Two-pass, streaming for feedback only.**

- While the hotkey is held, a streaming model feeds partial text to the overlay.
  It is never pasted and never stored.
- On release, the existing offline pipeline runs unchanged and produces the text
  that gets pasted.

This is worth building because it fixes a real complaint the category has:
between releasing the key and seeing the paste there is a silent gap, and
nothing on screen proves the app heard anything. Partials fill that gap with the
only evidence that matters.

**Lowercase the partials for display.** All-caps text replaced a moment later by
properly-cased text reads as a glitch. Rendering partials lowercase makes the
substitution look like refinement rather than correction.

**Cost to the user: a 296 MB download**, opt-in. That is most of the objection.
It should be a setting that defaults to off, described as "show words as you
speak", not a second model in the Models tab, because nobody choosing a *model*
wants to reason about a second one running alongside their first.

## Not doing

- **Streaming as the transcription path.** Points 3 and 4 rule it out: it would
  trade accuracy for immediacy on the one output the user keeps.
- **Apple SpeechAnalyzer sidecar.** Not evaluated. It would mean a second
  recognition stack, macOS-only, for a feature the shipped stack already does at
  40x real time. Revisit only if the download size proves to be the blocker.
- **Rescoring partials against the offline result.** Considered and rejected as
  premature: the offline text simply replaces the partials, and reconciling them
  word by word solves a problem nobody has demonstrated.
