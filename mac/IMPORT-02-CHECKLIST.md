# Inkwell 0.2 import from the app: checklist (Mac)

The core's tests prove `import.check` and `import.run` on synthetic 0.2 folders, and the Swift
tests prove what the first run and Settings say for each answer. What only a Mac with a real 0.2
folder can show is the whole path: the step, the button, the Library and dictation afterwards.
The hand-run check of the importer itself is `core/crates/ink-store/IMPORT-CHECKLIST.md`.

The app reads 0.2's folder (`~/Library/Application Support/com.inkwell.app`) read-only and never
changes it. It writes into the 1.0 library, once: a second import is refused.

## Setup

1. Quit Inkwell 0.2 (menu bar icon > Quit).
2. Pick where to try it. The import writes into the 1.0 library at its default place: a library
   moved with `INK_DATA_DIR` never looks for 0.2's data, on purpose. So either use a macOS user
   account made for testing, with a copy of the 0.2 folder (without `models/`) at that account's
   `~/Library/Application Support/com.inkwell.app`, or back up
   `~/Library/Application Support/Inkwell` first.
3. Note the source's sha256 sums: `cd ~/Library/Application\ Support/com.inkwell.app && shasum -a 256 *.json *.db`.

## A. The first run offers it

With a 1.0 library that has not finished its first run:

- [ ] After the permissions, a step "Your Inkwell 0.2 history" says in words what 0.2 left
      (dictations, snippets, modes, your settings), with **Import** and, at the bottom, **Not now**.
- [ ] **Not now** goes on to Polish and imports nothing. **Back** returns to the step.
- [ ] **Import**: the line reads "Bringing over what Inkwell 0.2 left…", then "Brought over from
      Inkwell 0.2: …", and the bottom button reads **Continue**. Record the counts.
- [ ] No system dialog about the keychain appears (the import only asks whether each provider has
      a key). If one does, press **Deny** and record it.
- [ ] If 0.2 used a key combination or separate presses, the key note appears under the line.
- [ ] If 0.2 held a right-hand modifier (right ⌥, say), the Ready step and the foot of Today
      name that key, not fn.

## B. After it

- [ ] The Library lists the imported dictations at once, without relaunching.
- [ ] Dictation uses what came over at once: 0.2's modifier key (if it had one) and a 0.2 snippet.
- [ ] Settings > Voice shows "Brought over from Inkwell 0.2: …" for this session; after a relaunch
      it shows no import row.
- [ ] The sha256 sums from Setup step 3 are unchanged.

## C. Later, from Settings

With a library that skipped the first run (press **Skip**) and holds no import:

- [ ] Settings > Voice shows the same row beside where the key note shows, with **Import**. It
      imports the same way, and the key note appears afterwards.
- [ ] Without leaving Settings, Snippets, Voice commands and Modes show what came over, and
      adding a snippet afterwards keeps the imported ones.

## D. Nothing to import

In an account without a 0.2 folder:

- [ ] The first run has no import step (and no dot for one), and Settings shows no row.
