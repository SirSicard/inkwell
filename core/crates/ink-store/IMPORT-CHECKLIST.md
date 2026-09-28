# Inkwell 0.2 import checklist (S2.9a)

CI proves the importer on synthetic 0.2 folders built from 0.2's own table definition and file
formats. What only a real folder can show is whether a real history of dictations and settings
comes across whole, so this list runs `examples/import_inkwell02.rs` on a **copy** of it.

The binary prints counts, file names, sizes and sha256 digests: never a dictation, a setting's
value or an API key. It asks the keychain only whether each provider has a key, through ink-llm's
attributes-only check; it never reads a key.

## Setup

1. **Quit Inkwell 0.2** (menu bar icon > Quit), and check it is gone:

       pgrep -lf Inkwell.app || echo "not running"

   A copy taken while it runs can catch a write halfway; the importer refuses such a copy
   ("an unfinished write sits beside it"). It also refuses any source that is not a plain file
   (a symbolic link, a folder), which a normal copy never produces.
2. Copy the data folder, without the models (they are large and not imported). Keep the copy
   **outside any git repository and any cloud-synced folder**: it holds your dictations.

       mkdir -p ~/inkwell-import-check
       rsync -a --exclude models \
         ~/Library/Application\ Support/com.inkwell.app/ ~/inkwell-import-check/source/

3. Build the binary, from `core/`:

       cargo build -p ink-store --example import_inkwell02

4. Count the source yourself, for comparison (read-only):

       sqlite3 'file:'"$HOME"'/inkwell-import-check/source/transcripts.db?mode=ro' \
         'SELECT count(*) FROM transcripts'
       cd ~/inkwell-import-check/source && python3 -c 'import json
       for f, k in [("dictionary.json", "entries"), ("snippets.json", "snippets"), ("modes.json", "modes"),
                    ("voice-commands.json", "commands"), ("app-styles.json", "rules")]:
           try: print(f, len(json.load(open(f))[k]))
           except FileNotFoundError: print(f, "absent")'

   Also note which providers show a stored key in 0.2's AI settings.

## A. Dry run

    ./target/debug/examples/import_inkwell02 \
      ~/inkwell-import-check/source ~/inkwell-import-check/new.sqlite --dry-run

**Must see:**

- `Keychain: asked which providers have a key (attributes only).` and **no system dialog**. If
  macOS asks to allow access to the "inkwell" keychain item, click **Deny** and record it: the
  existence check is supposed to be promptless, so that is a finding for ink-llm.
- Under `Dry run`: `dictations`, `dictionary entries`, `snippets`, `modes`, `voice commands` and
  `app style rules` equal your counts from Setup step 4; `linked keys` equals the number of providers with a key in 0.2;
  `settings` is the number of fields in `settings.json` that 0.2 itself uses (up to 20).
- `Sources byte-identical: yes`, `RESULT: PASS`, and no `new.sqlite` created.
- A `Dictation key:` line. With a modifier key in 0.2 (fn, right Command, right Option, right
  Control), it names the 1.0 key it becomes. With a key combination (0.2's default ⌘⇧Space) or any
  other key, it says **NOT carried over** and that 1.0 keeps fn. If 0.2 started and stopped on
  separate presses, a second line says 1.0 is hold to talk. Record which line you saw.

**Record** the `Left behind` lines. `plaintext key fields in settings.json` other than `none`
means an early build wrote those keys to disk in plain text: they are not imported, but rotate
them at the provider. (Starting 0.2 once removes them from its own `settings.json`.)

## B. Import

The same command without `--dry-run`:

    ./target/debug/examples/import_inkwell02 \
      ~/inkwell-import-check/source ~/inkwell-import-check/new.sqlite

**Must see:**

- Every row of the table ends in `ok`: the source count, what the import says it wrote, and the
  store counted again without the importer (`linked keys` there is what ink-llm itself finds in
  the keychain now).
- `Sources byte-identical: yes`, with the same digests as before, and no file added.
- `RESULT: PASS` and exit status 0 (`echo $?`).
- `dictation key in the new store:` the key the dry run named, or `not set (1.0's default, fn)`
  when it was not carried over.

A second run with the same `new.sqlite` is refused by the binary; a second import into one store
is refused by the importer itself (covered in CI).

## C. Spot check

The three latest dictations, as the new store holds them:

    sqlite3 ~/inkwell-import-check/new.sqlite "SELECT datetime(ended_at_unix_ms / 1000, \
      'unixepoch', 'localtime'), length(s.text) FROM record r JOIN segment s ON s.record_id = r.id \
      ORDER BY r.started_at_unix_ms DESC LIMIT 3"

**Must see:** the same times, to the second, as the three latest entries in 0.2's history, and
lengths that match their text. 0.2 saved local times without a zone, so the import reads them
in this Mac's current zone: entries dictated while the Mac was set to another zone come out
shifted by the difference.

## D. In the app: snippets, voice commands and the key

The new store is a whole 1.0 library: open the ad-hoc app on it (`mac/scripts/build-mac.sh`
first). It is your data, so keep it where it is, outside any repository.

    mkdir -p ~/inkwell-import-check/app
    (cd ~/inkwell-import-check && for f in new.sqlite*; do cp "$f" "app/library.sqlite${f#new.sqlite}"; done)
    INK_DATA_DIR=~/inkwell-import-check/app mac/build/Inkwell.app/Contents/MacOS/Inkwell

**Must see:**

- Settings > Snippets lists 0.2's snippets (the count from Setup step 4), in 0.2's order, with
  "Brought over from Inkwell 0.2.", and switched-off ones still off.
- Settings > Voice commands lists 0.2's commands with 0.2's switch and wake word. Commands this
  version does not do (open a URL or an app, undo, switch model, pause) say "Not available in this
  version".
- Settings > Voice > Dictate shows the key the import line named (or fn). When 0.2's key did not
  carry over, a note under the keys says so, naming the old keys (for example ⌘⇧Space); "Got it"
  removes it for good. With a key that carried over and no toggle, there is no note.
- Settings > AI: "Polish my words" is **off**, whatever 0.2 had: an import never turns polish on.
- Settings > Voice > "Edit a selection" is **Off**: 0.2's edit key is not carried over.
- With dictation on (and Accessibility for this build): dictate one of your snippet triggers in
  TextEdit; it expands. If 0.2's voice commands were on, say the wake word and one of your
  "Type text" commands: its text goes in.

## E. Clean up

Both the copy and the new store hold your dictations:

    rm -r ~/inkwell-import-check

## What the import leaves behind by design

- Each dictation's raw (pre-cleanup) text, its style and its model name: the 1.0 store has no
  place for them yet. They stay in 0.2's `transcripts.db`.
- Settings, snippets, voice commands and app style rules land under `import.inkwell-0.2.*` in the
  1.0 store, in 0.2's format. Dictation and Settings use the dictionary, snippets, modes and voice
  commands from there until the first change in Settings saves 1.0's own copy (the imported
  document stays as it was written).
- The dictation hotkey is the one 1.0 setting an import writes, and only when 1.0 can hold it and
  no key was chosen in 1.0 yet. Polish and the voice-edit key are never set by an import.

## Record

| Date | Dictations | Dictionary | Snippets | Modes | Settings | Voice commands | App styles | Keys | Dictation key line | Identical | Dialog? | In the app (D) | Result |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| | | | | | | | | | | | | | |
