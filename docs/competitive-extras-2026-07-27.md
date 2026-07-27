# Competitive extras and product analytics

*2026-07-27. Field scan of ~18 dictation apps, plus an audit of what Inkwell
actually ships. Recommendations are grounded in the code and in the nine real
transcripts now in the local history, not in general advice.*

---

## 0. Headline

Inkwell's feature surface is already at or above the paid field. The gaps are not
capability, they are **three specific behaviours the whole category has converged
on** (modes, voice editing, learning from corrections) and **two cheap wins the
category leader is being publicly asked for and has not shipped**.

The strongest single finding: your own nine transcripts contain the evidence for
two fixes. "Claude" was transcribed as *Claud*, *Cl*, and *Claude* across four
attempts, and "Um" passes through untouched into the pasted text.

---

## 1. What competitors ship as extras

Scanned: superwhisper, Wispr Flow, VoiceInk, MacWhisper, Willow, Monologue,
Typeless, Aqua, Alter, Raycast Dictation, Voibe, Amical, Paraspeech, Spokenly,
SpeakMac, Handy, Hex, OpenWispr, Whispering.

### Converged on, and Inkwell lacks

| Extra | Who ships it | What it is |
|---|---|---|
| **Modes** | superwhisper, VoiceInk, Raycast, Amical | A named bundle: model + prompt + formatting + per-app activation + its own shortcut. superwhisper's is called "best in class" and is the main reason people pay $249. |
| **Voice editing / Command Mode** | Wispr Flow, Spokenly | Select text, hold key, say "make this shorter" — it rewrites in place rather than inserting. |
| **Learning from corrections** | Willow ("smart memory", "vocabulary learning"), VoiceInk ("dictionary training") | The app notices what you fix and stops making that mistake. |
| **Filler-word cleanup** | Typeless, VoiceInk, Willow | Strips *um*, *uh*, repetitions, false starts before pasting. |
| **Mid-sentence correction** | Typeless | "…meet at three, no wait, four" resolves to "four". |
| **Screen / selection context** | Alter, Aqua, Monologue, VoiceInk (OCR) | Reads what is on screen or selected to inform formatting. |
| **Shell command triggers** | superwhisper | Pipe the transcript through a script after transcription. |
| **Translation** | Typeless, Paraspeech | Dictate in one language, insert another. |
| **Sync across devices** | Wispr Flow | Settings and dictionary follow you. |

### Asked for across the field and largely unshipped

These come from superwhisper's public request board (vote counts) and repeated
review complaints. They are the clearest openings, because even the leader has
not closed them:

- **Pause / resume a recording** — 156 votes, and independently the single named
  complaint in an open-source roundup. Nobody ships it well.
- **Automatic language detection** — 94 votes.
- **Cross-device sync** — 286 votes (not viable for a local-first app, and worth
  declining out loud rather than quietly).
- **Manual transcript cleanup** is the most-cited complaint category overall,
  especially on long recordings. That is the filler-word and false-start problem.

### Where Inkwell already matches or beats the field

Worth knowing so these are not rebuilt or undersold:

- **Twelve models with in-app download and switching.** Only Handy and Spokenly
  are comparable; most paid apps ship one family.
- **File transcription across 12 container formats** with TXT/SRT/JSON/CSV
  export. MacWhisper charges €59 largely for this.
- **Per-app style rules**, now on macOS bundle IDs.
- **Snippets with `{date}` / `{time}` / `{clipboard}` interpolation.** Rare.
- **Voice commands with a wake prefix.** Rare outside superwhisper.
- **Searchable SQLite history.**
- **BYOK polish across five providers**, with the key in the OS keyring.
- **A real level meter, six overlay positions, an appearance setting.** Wispr
  hardcodes its pill position and someone shipped a third-party utility purely
  to move it.

---

## 2. Analytics on what we have

### Measured

| | |
|---|---|
| Rust | 5,646 lines, 27 modules |
| TypeScript | 3,885 lines |
| Tauri commands | 38 |
| Settings keys | 15 |
| Tests | 101 passing, plus 2 integration tests behind `--ignored` |
| Real transcripts | 9, averaging 4.3s, longest 22.6s / 333 chars |

### The pipeline, in execution order

`resample → VAD → transcribe → voice-command check → style → dictionary →
snippets → polish → paste`

That order is right, and it is worth stating why: dictionary runs after style so
corrections are not re-cased, and polish runs last so it sees the finished text.

### What your own transcripts prove

**1. Proper nouns are being mangled, and nothing learns.** Four attempts at the
same word produced *Claud*, *Cl*, *Claude*, *Claud*. The dictionary feature that
would fix this exists, is empty, and requires you to know to go and fill it in.
Every competitor with "learning" is solving exactly this.

**2. Filler words reach the clipboard.** "Hello. Um Can you actually hear me?"
was pasted verbatim. No stage strips them: `style.rs` handles casing and
punctuation only. This is the category's most-cited complaint and Inkwell has
none of the three defences (filler stripping, false-start resolution, polish is
off by default and needs a key).

**3. Long-form works.** The 22.6s / 333-char transcript came through coherent,
which means the 15s chunking and the seam merge are doing their job.

### Delete or demote

- **`recording_mode` toggle vs push-to-talk** is fine, but **`advanced_mode` as a
  global switch is the wrong shape** now that navigation is a grouped sidebar.
  It exists to hide complexity; Modes would do that better and per-context.
- **Three text styles** (formal / casual / relaxed) are a weak version of Modes.
  They cannot carry a prompt, a model, or a shortcut. When Modes lands, styles
  should become a property *of* a mode, not a parallel concept.
- **`debug_save_audio`** should stay, but it belongs in a Troubleshooting section
  rather than General.
- **Voice commands** are off by default and duplicate what a Command Mode would
  do better. Keep the wake-word engine; consider folding the feature into Modes
  rather than maintaining a separate concept.

### Add, ranked by value over effort

1. **Pause / resume recording.** 156 public votes on the leader's board, no good
   implementation anywhere, and Inkwell's architecture makes it nearly free: the
   capture buffer already accumulates behind an `AtomicBool`. Toggling that flag
   without ending the session *is* pause.
2. **Filler-word and false-start cleanup**, as a style-layer stage that runs with
   no API key. This is the most-complained-about thing in the category and it is
   pure text processing, the part of the codebase that is already pure and
   well-tested.
3. **Learn from corrections.** After a dictation, offer a one-click "fix this
   word everywhere" that writes a dictionary entry. Cheap, and it turns an empty
   feature into a self-filling one.
4. **Modes.** The big one, and the real competitive gap. It should absorb styles,
   per-app rules, and advanced mode rather than sit beside them.
5. **Voice editing (Command Mode).** Select text, hold key, speak an
   instruction. Needs polish to be configured, so it lands after BYOK is proven.
6. **Announce automatic language detection.** Parakeet V3 already does this
   across 25 European languages. It is a shipped capability the UI never claims
   and that superwhisper users are actively requesting.

### Decline, out loud

- **Cross-device sync** (286 votes) contradicts local-first. Say so on the page
  rather than leaving it as an apparent oversight.
- **Meeting mode / diarization.** A different product; MacWhisper owns it.
- **Mobile.** Not a desktop paste tool.

---

## 3. What I would do next

Given the app now works end to end and the field is mapped:

**Pause, filler cleanup, and dictionary-from-correction** are all small, all land
in already-tested pure code, and all three answer the loudest complaints in the
category. That is one focused session.

**Modes** is the strategic one and deserves its own. It is the difference between
"a good free dictation app" and "the thing people choose over a $249 licence",
and it is the only item here that changes the product's shape rather than adding
to it.
