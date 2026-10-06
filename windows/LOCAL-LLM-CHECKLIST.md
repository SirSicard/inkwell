# This PC's language model checklist: download, Use, polish's one tap, Remove

What an SSH session cannot check: the first-run sheet, Settings > AI's "On this PC" rows, the
consent step, Narrator, a real 2.3 GB download over a real network, and a full disk. About forty
minutes at each PC (one with a graphics card, one with an Intel iGPU, one with no GPU), signed in,
with a scratch library (`INK_DATA_DIR`). Note the date, the PC, the Windows build and the commit
under Result. Downloading the model needs the maintainer's OK, as every model download does.

Automated checks cover the logic behind some lines: `LocalLlmModelTests` (Use, then polish's one
tap, as the core answers), `CatalogueModelTests` (the language row) and `CloudModelTests` (this
PC's model in the picker). Nothing else here is tested by machine.

## First run

- [ ] Models step: "Polish, edit and summaries on this PC" is unticked, with the line "Qwen3 4B
      Instruct · Apache-2.0 · 2.32 GB · from huggingface.co" and what it adds. A free-space line
      ("… free on C:") is under Download.
- [ ] Tick it: Download's title adds 2.32 GB to the total; Narrator reads every model it fetches.
      Untick it: the total drops back. Nothing downloads until Download is pressed.
- [ ] Press Download and go on: the Polish step says "Qwen3 4B Instruct is downloading (… of …)",
      its switch is off and can't be switched, and the Groq disclosure reads "Or use Groq's free
      model".
- [ ] Wait on the Polish step until the download ends: the line changes to "… is on this PC: turn
      polish on above…" and the switch works without leaving the step.
- [ ] Switch polish on: one card, "Turn on polish?", "… It uses Qwen3 4B Instruct, on this PC, so
      your words stay on this PC.", focus on Turn On Polish. One press turns it on; Narrator reads
      the heading and the message.
- [ ] Leave it unticked: the Polish step is as before (Groq's free model, no line about this PC).

## Settings > AI

- [ ] The picker lists None, "On this PC: Qwen3 4B Instruct", then the own-key providers. The
      caption under it says what this PC has: "no language model of its own" never shows while the
      core offers this PC's model.
- [ ] Not downloaded: its line, "Not downloaded…", the free space, Download. Use is off.
- [ ] Download: the progress bar and "… of …" move; Cancel shows. Cancel: "Stopping the
      download…", then "Stopped. Download picks up where it left off." Download again starts from
      where it stopped (watch the first progress figure), not from 0.
- [ ] Downloaded with no provider chosen: the status says "In use: Qwen3 4B Instruct, on this PC, as
      no provider is chosen"; polish, voice edit and summaries can be switched on, each with its
      own one-tap step.
- [ ] Try it: "Loading Qwen3 4B Instruct and asking it…", then "… loaded in N s and answered in N
      s." A second Try it shows a load near 0. Write both numbers under Result for each PC.
- [ ] With Groq chosen and polish on for Groq: pick On this PC, Use. Local-only mode turns back on,
      then the one-tap step appears ("Turn On Polish"). Cancel: the model stays chosen, polish reads
      "Paused: polish now uses Qwen3 4B Instruct, on this PC…". Switch polish on: the step asks
      again; Turn On Polish: polish reads "On, with Qwen3 4B Instruct, on this PC".
- [ ] With polish already on for this PC, Use asks nothing.
- [ ] With this PC's model chosen and a Groq key saved, Groq is never called (Local only stays on).

## Settings > Models

- [ ] The language model is a row like the speech models, with its name; every row has Download,
      Cancel while it downloads, or Remove… once installed. The free space is under the list.
- [ ] Remove… asks "Remove Qwen3 4B Instruct from this PC?", naming its size and that nothing takes
      its place; Cancel is focused and Enter removes nothing. Remove deletes it: the row reads not
      installed, Storage's Models size drops, Settings > AI says "This PC's model is chosen, but it
      isn't downloaded" (if it was chosen) and polish reads no model. Nothing falls through to Groq.
- [ ] Remove while dictating with polish on this PC's model: the row says "… is in use, so it
      wasn't removed. Try again once it's done." Remove a speech model during a meeting: the same.
- [ ] A full disk: fill the models' volume to under 3.4 GB free (a scratch VHD as the library's
      drive is easiest), then Download: "Needs 3.40 GB; … free on X:" with Retry, and nothing in
      the models folder changed.
- [ ] A mode pinned to this PC's model: after Remove, its row says "Its model, this PC's language
      model, isn't available now…"; the editor's list shows "this PC's language model (not
      available)". Download again: the mode polishes again with no step.

## Narrator and size

- [ ] Narrator names: the picker's "On this PC: Qwen3 4B Instruct"; Download ("Download Qwen3 4B
      Instruct, 2.32 GB, from huggingface.co"), Cancel ("Cancel downloading …"), Remove ("Remove …
      from this PC"), Retry, the progress bar ("Downloading …"), Try it ("Try Qwen3 4B Instruct on
      this PC"), and each first-run choice with its lines.
- [ ] At 720 epx wide, Settings > AI's "On this PC" rows and Settings > Models' rows fit their cards:
      nothing cut, buttons not pushed out, lines wrap.
- [ ] The polish latency on each PC: dictate about 50 words with polish on this PC's model, five
      times; note how long each takes to type, and whether any went in unpolished (timed out).

## Result

- Date, PC, Windows build, commit:
- Try it (load, answer) per PC:
- Polish latency per PC:
- Notes:
