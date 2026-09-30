# Inkwell 0.2 import from the app: checklist (Windows)

The core's tests prove `import.check` and `import.run` on synthetic 0.2 folders (on this PC too),
and `Inkwell.Core.Tests` prove what the first run and Settings say for each answer. What only a
desktop session with a real 0.2 folder can show is the whole path: the step, the button, the
Library and dictation afterwards. Nothing here was run from an SSH session.

The app reads 0.2's folder (`%APPDATA%\com.inkwell.app`, where Inkwell 0.2 kept its data) read-only
and never changes it. It writes into the 1.0 library, once: a second import is refused.

## Setup

1. Quit Inkwell 0.2 (notification area icon > Quit).
2. Pick where to try it. The import writes into the 1.0 library at its default place
   (`%LOCALAPPDATA%\Inkwell`): a library moved with `INK_DATA_DIR` never looks for 0.2's data, on
   purpose. So either use a Windows account made for testing, with a copy of the 0.2 folder
   (without `models`) at that account's `%APPDATA%\com.inkwell.app`, or back up
   `%LOCALAPPDATA%\Inkwell` first.
3. Note the source's hashes: `Get-FileHash $env:APPDATA\com.inkwell.app\*.json, $env:APPDATA\com.inkwell.app\*.db`.

## A. The first run offers it

With a 1.0 library that has not finished its first run:

- [ ] After the permissions, a step "Your Inkwell 0.2 history" says in words what 0.2 left
      (dictations, snippets, modes, your settings), with **Import** and, at the bottom, **Not now**.
      The dots have one for it.
- [ ] **Not now** goes on to Polish and imports nothing. **Back** returns to the step.
- [ ] **Import**: the line reads "Bringing over what Inkwell 0.2 left…", then "Brought over from
      Inkwell 0.2: …", and the bottom button reads **Continue**. Record the counts.
- [ ] If 0.2 used a key combination, Fn, or separate presses, the key note appears under the line.
- [ ] Narrator reads the step's heading, the line and the Import button.

## B. After it

- [ ] The Library lists the imported dictations at once, without relaunching.
- [ ] Dictation uses what came over at once: 0.2's key (if 1.0 can hold it) and a 0.2 snippet.
- [ ] Settings > Voice shows "Brought over from Inkwell 0.2: …" for this session; after a relaunch
      it shows no import row.
- [ ] The hashes from Setup step 3 are unchanged.

## C. Later, from Settings

With a library that skipped the first run (press **Skip**) and holds no import:

- [ ] Settings > Voice shows the same row under the keys, beside where the key note shows, with
      **Import**. It imports the same way, and the key note appears afterwards.

## D. Nothing to import

In an account without a 0.2 folder:

- [ ] The first run has no import step (and no dot for one), and Settings shows no row.
