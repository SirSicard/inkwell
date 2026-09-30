# Own-key language model checklist (Windows)

What an SSH session cannot check: Windows Credential Manager in your own session, a real API key
against a real provider, and Settings > AI on screen. About thirty minutes at the PC, signed in,
with one real key (OpenAI or Anthropic) and a little credit on it. Note the date, the Windows build
and the commit under Result.

Automated checks cover the logic behind each line: the commands and the core's side
(`cargo test -p ink-ffi --test cloud`: every provider listed, a key only in the key store and in no
event or library file, a cloud provider chosen only with local-only mode turned off by the shell,
nothing sent without each feature's consent for that endpoint, dictation polish through the chain,
the key test), privacy under the most verbose logger (`--test cloud_privacy`: neither the key nor
the words in a log line, an event or an error), the providers and the local-only guard
(`cargo test -p ink-llm`), and the screen's model (`dotnet test`: `CloudModelTests`).

## Setup

In a **Developer Command Prompt for VS** (x64), from the repository. No engines are needed for
this checklist (dictation needs them: build as in `S3.5a-CHECKLIST.md` for section 5).

```
cd core
cargo build --release -p ink-ffi --lib
cd ..\windows
dotnet publish Inkwell\Inkwell.csproj -c Release -o %TEMP%\inkwell-app -p:InkCoreDir=%CD%\..\core\target\release\
set INK_DATA_DIR=%TEMP%\inkwell-byok
%TEMP%\inkwell-app\Inkwell.exe
```

## 1. Credential Manager

- [ ] From a terminal at the desktop (not SSH), in `core`:
      `cargo test -p ink-llm --lib -- --ignored the_os_existence_check_agrees_with_what_was_stored`:
      1 passed. It writes, finds and deletes a test value under `inkwell-selftest-provider`.
- [ ] Control Panel > Credential Manager > Windows Credentials: no `inkwell-selftest-provider`
      entry is left.

## 2. The key

- [ ] Settings > AI > Language model reads "None: nothing leaves this PC", and the line under it
      "No language model is in use. Local-only mode is on." Polish, Voice edit and Summaries and
      Ask are off and cannot be switched ("No language model is available on this PC ...").
- [ ] Pick **OpenAI** (or **Anthropic**). Nothing else changes yet; the key line reads "No key is
      stored yet."
- [ ] Paste the key into **API key**: it shows as dots. **Save key**: the box empties at once,
      and the line reads "A key is stored in Windows Credential Manager."
- [ ] Credential Manager > Windows Credentials shows `openai.inkwell` (or `anthropic.inkwell`).
- [ ] In PowerShell, with the first 12 characters of your key in `$k`:
      `Get-ChildItem $env:TEMP\inkwell-byok -Recurse -File | Select-String -SimpleMatch $k`
      finds nothing: the key is in none of Inkwell's files.
- [ ] Paste `not a key` and Save key: "Couldn't do that: the key holds a space or a character no
      API key has." The stored key is untouched.

## 3. Use (local-only mode)

- [ ] Before pressing it, the note under the picker says "Using OpenAI turns local-only mode off,
      so the features below can send to OpenAI. Each one still asks before it sends anything."
- [ ] **Use OpenAI**: the line reads "In use: gpt-4o-mini at OpenAI. Local-only mode is off."
      The three switches below can now be used, and all three are still off.

## 4. Test

- [ ] **Test**: "Asking OpenAI…", then "OpenAI answered with gpt-4o-mini." within a few seconds.
- [ ] Save a wrong key (for example your key with its last letter changed), **Test**: "Couldn't get
      an answer: the provider refused the key." in the alert colour. Save the right key again and
      Test: it answers.
- [ ] Model: type a model id that does not exist (`no-such-model`), **Use OpenAI**, **Test**:
      "Couldn't get an answer: the provider does not know this model or address." (or the
      provider's 400: "... the provider said 400"). Pick the default model from the list again,
      Use, Test: it answers.

## 5. Each feature asks first

Dictation needs the engines (build as in `S3.5a-CHECKLIST.md`, same Settings steps as above).

- [ ] Switch **Polish my words** on: the consent step names "gpt-4o-mini (openai)" and says your
      words leave this PC for it; Enter lands on **Cancel**. Cancel: polish stays off. Dictate a
      sentence in Notepad: it goes in as you said it (nothing was sent).
- [ ] Switch it on again and press **Send to gpt-4o-mini (openai)**: the line reads "On. Your words
      go to gpt-4o-mini (openai) before they are typed." Dictate: the text is tidied.
- [ ] **Voice edit**: the same step, for the selection and what you say. Allow, select a sentence
      in Notepad, hold the edit key, say "make it shorter": the selection is replaced.
- [ ] **Summaries and Ask**: the same step, for the meeting's transcript. Allow. Once meetings run
      on this PC (S3.5b), a short meeting gets a summary and Ask answers.

## 6. Another provider, a deleted key, none

- [ ] Pick **Anthropic** (with a key of its own saved) and **Use Anthropic**: each switch that was
      on now reads "Paused: ... would now send your words to ..." and nothing is sent until you
      turn it on again and allow Anthropic. (Skip this without a second key.)
- [ ] Pick the provider in use, **Delete key**: "No key is stored yet."; its entry is gone from
      Credential Manager; the line reads "... is chosen, but no key is stored, so nothing can use
      it."; Test says "Couldn't test it: no key is stored for it".
- [ ] Pick **None** and **Stop using a language model**: the line reads "No language model is in
      use. Local-only mode is on." and the switches cannot be used.
- [ ] Choose the provider again (key saved, Use), quit from the tray and start again: Settings > AI
      shows the same provider, model and "Local-only mode is off."
- [ ] Narrator: the picker, the key box ("API key"), the model box, Use, Test and each line are
      read out; the key is never read out.

## Result

Date, Windows build, commit:

Notes:
