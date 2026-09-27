# Focus checklist: the Drop

About five minutes, by hand: whether keystrokes land where the user is typing cannot be checked
from a script. The Drop must never take focus: not when it appears, not while it changes state,
not when it hides and comes back, and not when it is clicked. Run it after any change to the Drop
(mac/Sources/Inkwell/Drop.swift) and note the date, the macOS version and the commit.

## Setup

- [ ] Build: `mac/scripts/build-mac.sh` (an ad-hoc build is fine: nothing here needs a
      permission).
- [ ] Open TextEdit with a new document.
- [ ] Start Inkwell with the Drop cycling through its states, one every 3 seconds:

      INK_DROP_DEMO=3 INK_DATA_DIR="$(mktemp -d)" mac/build/Inkwell.app/Contents/MacOS/Inkwell

      The cycle is dictating, meeting, blotting, problem, then idle (the Drop hides), and again.
      INK_DATA_DIR keeps the run away from your library.
- [ ] Inkwell's window opens at launch and takes focus once, as any app does when started. Close it
      (Command-W) or leave it; then click into the TextEdit document.

## Typing while the Drop cycles

- [ ] Type continuously for at least two full cycles (about 30 seconds): every character lands in
      TextEdit, and none is lost as the Drop appears, changes or hides.
- [ ] TextEdit's window title and the text cursor stay active (not greyed) throughout.
- [ ] The menu bar keeps showing TextEdit's menus: Inkwell never becomes the active app.
- [ ] The Drop sits at the bottom centre of the screen, above other windows, and shows each state:
      "Dictating", "● REC", "Blotting", "Far end silent" (seal-red border), then disappears.
- [ ] Command-Tab: Inkwell is not moved to the front of the list by the Drop.

## Clicking the Drop

- [ ] While the Drop shows, click on it (the ink and the text). TextEdit stays active and its
      cursor keeps blinking; typing afterwards still lands in TextEdit.
- [ ] Command-` (cycle windows): the Drop is never one of the windows cycled to.

## Spaces and full screen

- [ ] Switch to another Space (Control-Right): the Drop follows and still takes no focus.
- [ ] Put TextEdit in full screen (Control-Command-F): the Drop shows over it, and typing still
      lands in TextEdit.

## Motion

- [ ] With Reduce Motion on (System Settings > Accessibility > Display > Reduce motion), the Drop's
      ink shows each state as a still drop and does not move; with it off, the ink moves while the
      Drop shows.

## Finish

- [ ] Quit Inkwell from its menu-bar item (or Control-C in the terminal).

## Result

Date, macOS version, commit:

Failures (the step, and what happened instead):
