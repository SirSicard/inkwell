# Mac input checklist (S2.1b)

The hotkey tap, synthetic input and Accessibility reads need the Accessibility permission, which CI
and an agent's shell do not hold, so they are checked by hand with `examples/input_check.rs`. The
unit tests cover every decision (token parsing, the tap's state machine, the paste sequence, the
restore rule) against mocks; this list covers what only a real session can show.

The binary never prompts for a permission and never prints clipboard or selection content: only
counts, timings and a fingerprint comparison.

## Setup

1. Build it, from `core/`:

       cargo build -p ink-platform-mac --example input_check

2. Grant **Accessibility** to the terminal app you run it from: System Settings > Privacy & Security
   > Accessibility. (The terminal is the process macOS asks about.)
3. Copy a recognisable line of text (a *sentinel*), for example `clipboard sentinel 42`. It must
   still be on the clipboard at the end.
4. Run the probe, which touches nothing:

       ./target/debug/examples/input_check --probe

   **Must see:** `accessibility granted, post events granted`; a 100 ms sleep measured at about
   100 ms (not about 2 ms, which would mean the clock is on the wrong timebase);
   `main run loop: serving`.

## A. Fn hold

1. Note the setting in System Settings > Keyboard > "Press 🌐 key to", and set it to **Do Nothing**.
2. Run `./target/debug/examples/input_check` (the default hotkey is `fn`, 3 cycles).
3. Open a new TextEdit document, click into it, hold Fn for about a second, release.

**Must see**, per cycle:

- `pressed (stamped N ms ago)` with N under about 50, then a `focus:` line naming
  `com.apple.TextEdit` with `secure input false`;
- `released (stamped N ms ago), held 1.0 s` (roughly how long you held it);
- `clipboard: same items as before`;
- `insert: Pasted in` roughly 350 to 450 ms;
- `Inkwell input check.` in the document **exactly once**.

Then set "Press 🌐 key to" to **Show Emoji & Symbols** and hold Fn again. **Record** whether the
emoji picker still opens. (The tap swallows the Fn events; this records whether that is enough to
stop the system's own Fn action. Either answer is a finding, not a failure.) Put the setting back.

## B. Paste matrix

For each app, click into a text field, hold the hotkey, release. Run with `--cycles 6` to do all
six in one go.

| App | Where | Must see |
|---|---|---|
| TextEdit | a document | |
| Slack | the message box (do not send) | |
| Chrome | any text field, or the address bar | |
| Terminal | a shell prompt | text on the prompt line, **not run** |
| Notes | a note | |
| Mail | the body of a new message (do not send) | |

**Must see** in every row: the text once, `insert: Pasted`, `clipboard: same items as before`. Fill
the table with the outcome and the time from the `insert:` line.

## C. Selection

In TextEdit, select one word, then hold and release the hotkey. **Must see:** `selection: N
characters` with N the length of the word (the insertion then replaces the word, as a paste
does). With nothing selected: `selection: none`.

## D. Other bindings

Run with each of these and hold and release once in TextEdit:

- `--hotkey right_option`
- `--hotkey right_command`
- `--hotkey ctrl+shift+space`: **must not** type a space into the document (the chord is swallowed)
- `--hotkey f13`, if the keyboard has one

And one that must be refused: `--hotkey left_option` prints `FAIL hotkey ... not supported here`.

## E. Secure Input

1. In Terminal, turn on Terminal > Secure Keyboard Entry.
2. Run `./target/debug/examples/input_check --timed 5` in Terminal and leave Terminal in front.

**Must see:** `secure input true` on the `focus:` line, `insert: Blocked`, nothing typed, and
`clipboard: same items as before`. Turn Secure Keyboard Entry off again.

A password field in a browser (a login form) should give the same result with `--timed 5`.

## F. Clipboard

At the end of the run, press Cmd+V in any text field. **Must paste the sentinel** from setup
step 3. Then copy something new, run one more insertion, and check that Cmd+V now pastes the new
copy (the restore puts back whatever was there, not the first sentinel).

## G. No permission, no prompt

1. Quit the binary. In System Settings > Privacy & Security > Accessibility, switch the terminal
   app off.
2. Run it with no arguments. **Must see:** `FAIL hotkey "fn": permission not granted:
   Accessibility`, and **no system dialog**.
3. Run `--timed 3`. **Must see:** `insert: FAILED ... permission not granted: Accessibility`, no
   dialog, and `clipboard: same items as before`.
4. Switch the terminal back on.

## Result

Green when A, B (all six rows), E, F and G pass as described. Record the section A emoji-picker
finding and any timings far outside the ranges above.
