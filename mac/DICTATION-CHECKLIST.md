# Checklist: dictation end to end

About forty minutes, by hand, plus the timed parts (section 6 and 7). What a script cannot check
is here: the keys (an event tap under Accessibility), the microphone, insertion into other apps,
and how it feels. Note the date, the macOS version and the commit at the top of your copy.

Automated checks cover the logic behind each line: the chain on mocks (`cargo test -p
ink-pipeline --test dictation_takes`: a second press never wipes the take, the 180 s watchdog,
voice edit, the polish switch, live words, the mic let go of while idle), the warm-up
(`--test warm`), dictation through the core on the mock platform (`cargo test -p ink-ffi --test
voice`), and the shell (`swift test --package-path mac --filter Dictation`).

## Setup

- [ ] Build the signed app with its engines (an ad-hoc signature is a new app to TCC every time,
      so the grants below would not stick):

      INK_SIGN_IDENTITY=<your Developer ID> NEMO_SPEECH_DIR=<NeMo prefix> \
        mac/scripts/build-mac.sh --engines

- [ ] Stage the dictation models from the bench data (symlinks, nothing downloaded):

      INK_BENCH_DIR=~/.blotter-bench scripts/stage-dictation-models.sh /tmp/inkwell-dictation-models

- [ ] Start it on a scratch library with those models:

      INK_DATA_DIR="$(mktemp -d)" INK_MODELS_DIR=/tmp/inkwell-dictation-models \
        mac/build/Inkwell.app/Contents/MacOS/Inkwell

- [ ] System Settings > Keyboard > "Press 🌐 key to": note your setting, and set it to
      **Do Nothing** for this checklist (put it back at the end).
- [ ] Copy a recognisable sentinel line (`clipboard sentinel 42`): it must still be on the
      clipboard at the end of every section.

## 1. The keys (TCC: Accessibility)

- [ ] Before granting anything: Settings > Voice says dictation needs "Type for you"
      (Accessibility), with an **Allow** button. Holding fn does nothing, and fn still works as
      usual (fn + arrow keys).
- [ ] Allow: macOS opens Accessibility; turn Inkwell on; come back to Inkwell. Within a few
      seconds Settings > Voice reads "Hold fn, speak, let go."
- [ ] Settings > Voice > Dictate: pick **Right Option**. At once: holding fn does nothing, holding
      right Option dictates. Pick **fn** again.
- [ ] Settings > Voice > Edit a selection: the list offers Off and every key but the dictation
      key. Leave it Off for now.
- [ ] A quick tap of fn (as in a shortcut) shows nothing and types nothing. fn + arrow keys still
      work in TextEdit.

## 2. Dictating into TextEdit (TCC: Microphone)

- [ ] New TextEdit document, caret in it. Hold fn and say a sentence for about three seconds.
      The first press asks for the microphone once ("Hear you"): allow it.
- [ ] While held (from about 0.2 s): the Drop appears at the bottom of the screen reading
      "Dictating · TextEdit · Default" (the app in front and its mode). With Parakeet installed,
      your words appear beside the ink as you speak, the newest two in grey italic (still wet);
      without it, "Listening". The ink moves with your voice.
- [ ] Typing in TextEdit while the Drop shows still goes to TextEdit (the Drop never takes focus).
- [ ] Let go: "Transcribing", then the sentence appears at the caret with a trailing space, and
      the Drop goes away. The clipboard still holds the sentinel.
- [ ] The microphone indicator (orange dot) stays on after the take. Dictate again within a
      minute, starting to speak exactly as you press: the first word is complete (the take keeps
      the 300 ms before the press).
- [ ] Leave it alone for 3 minutes: the microphone indicator goes off (the mic is let go of). The
      next dictation works; its very first syllable may be clipped (the mic starts at the press).
- [ ] Hold fn for about 0.25 s saying "hi": the Drop says "Too short / Try again". Nothing typed.
- [ ] Hold fn in silence for two seconds: "No speech heard" (never made-up words).
- [ ] Two takes back to back: let go and at once hold again: both go in, the first complete.

## 3. Modes and polish

- [ ] Settings > Modes shows the modes; with none stored, one "Default" row with Polish.
- [ ] If you have 0.2's modes imported (a Chat mode naming Slack): dictate in Slack. The Drop
      reads "Dictating · Slack · Chat" and the text is written casually.
- [ ] Settings > AI > Polish my words: on (needs Apple Intelligence). Dictate "um so the the
      meeting is at noon": the fillers go and the sentence is tidied. Off: it goes in as said.
- [ ] If polish is ever slow, the Drop says "Polish took too long / Typed as you said it" and the
      text goes in as said (never waits more than 10 s).

## 4. Voice edit

- [ ] Settings > Voice > Edit a selection: **Right Command**. Settings shows it held.
- [ ] In TextEdit select a sentence. Hold right Command, say "make it more formal", let go. The
      Drop reads "Editing the selection", "Say what to change", then "Rewriting"; the selection
      is replaced by the rewrite (no extra space). The Library has no new entry for it.
- [ ] With nothing selected: the Drop says "Select some text first".
- [ ] Right Command in shortcuts (right ⌘ + C / V) still copies and pastes, and shows nothing.
- [ ] With Apple Intelligence off: an edit says "Editing needs Apple Intelligence" and leaves the
      selection alone.
- [ ] Set it back to Off: right Command is an ordinary key again.

## 5. Where it cannot type, and permissions taken away

- [ ] Terminal > Secure Keyboard Entry on, caret in Terminal, dictate: the Drop says "Secure
      input is on" and nothing is typed; the words are in the Library. Turn it off again.
- [ ] System Settings: turn Inkwell's Microphone off. Hold fn: the Drop says "Couldn't open the
      microphone". Turn it back on: the next take works.
- [ ] System Settings: turn Inkwell's Accessibility off while it runs. Today says the dictation
      key stopped working; Settings > Voice says it needs "Type for you". Turn it back on, come
      back to Inkwell: dictation works again without a restart.

## 6. The paste matrix (six apps)

Dictate one sentence into each, the caret in a text field. Check: the text lands at the caret,
once, with its trailing space; the clipboard still holds the sentinel afterwards; nothing else
happened in the app (no shortcut fired).

| App | Where | Text at the caret, once | Clipboard restored | Notes |
|---|---|---|---|---|
| TextEdit | a document | [ ] | [ ] | |
| Slack | the message box | [ ] | [ ] | |
| Chrome | a text field (a search box, a Google Doc) | [ ] | [ ] | |
| Terminal | the prompt | [ ] | [ ] | |
| Notes | a note | [ ] | [ ] | |
| Mail | a new message body | [ ] | [ ] | |

## 7. Ten timed takes: key-up to text in TextEdit

The harness times from the key coming up to the text arriving in the app, as you feel it. It
reads only the document's length, never its text.

- [ ] Build it: `cd core && cargo build --release -p ink-platform-mac --example dictation_timing`
- [ ] Give your terminal app **Input Monitoring** (it watches the key without taking it from
      Inkwell) and **Accessibility** (it reads TextEdit's length): System Settings > Privacy &
      Security. Nothing prompts.
- [ ] With Inkwell running and the caret in an empty TextEdit document:

      ./target/release/examples/dictation_timing --key fn --runs 10

- [ ] Dictate ten sentences of about five seconds each, letting go right after the last word.
      Record the summary line: p50 ______ ms, p95 ______ ms, max ______ ms.
- [ ] The target for the core alone (section 8) is p50 ≤ 350 ms and p95 ≤ 700 ms; this adds the
      capture path, the tap, the paste and TextEdit drawing it. Note anything above 1 s.
- [ ] Take your terminal's Input Monitoring away again if you do not want it kept.

## 8. The latency measurement (a quiet Mac, nothing else running)

Not by hand, but it needs a quiet Mac, so it is here. About 45 minutes; it refuses while the
machine is busy.

- [ ] `INK_BENCH_DIR=~/.blotter-bench scripts/dictation-latency.sh`
- [ ] Record from its summary: warm p50 ______ / p95 ______ ms (target 350 / 700); the first take
      after 240 s idle with the warm-up ______ ms and without ______ ms.

## At the end

- [ ] Put System Settings > Keyboard > "Press 🌐 key to" back as it was.
- [ ] The sentinel is still on the clipboard.
