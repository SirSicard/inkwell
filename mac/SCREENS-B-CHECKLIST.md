# Checklist: Live, Owed, Settings and the first run

About twenty minutes, by hand. What a script cannot check is here: permissions the system grants
(TCC), how the screens look and read against the design canvas, and VoiceOver. Run it after a change
to `mac/Sources/Inkwell/Screens/` and note the date, the macOS version and the commit.

Automated checks cover the logic behind each line (`swift test --package-path mac`): the
permission cards' reaction to a probe change, the Polish toggle, modes without bundle ids, owed
grouping, the notes' saving, the ledger and the question stack, and every screen command against
the real core.

## Setup

- [ ] Build: `mac/scripts/build-mac.sh`. The ad-hoc build is fine for everything except section 2,
      which needs the signed build (`INK_SIGN_IDENTITY` set): an ad-hoc signature is a new app to
      TCC every time.
- [ ] Start it on a scratch library, with a meeting replayed from the fixtures once the engines are
      up (30 seconds of audio):

      INK_DATA_DIR="$(mktemp -d)" \
      INK_REPLAY_MEETING="$PWD/fixtures/ami/IS1009a-mic.wav,$PWD/fixtures/ami/IS1009a-far.wav" \
      mac/build/Inkwell.app/Contents/MacOS/Inkwell

## 1. First run

- [ ] A fresh library opens the first-run sheet over the window: four steps shown as dots.
- [ ] Permissions step: four cards, "Hear you", "Hear the others", "Type for you", "Know your
      meetings", each with its state. Nothing is asked for until you press Allow.
- [ ] Polish step: the switch is off and cannot be turned on if Apple Intelligence is off or not
      on this Mac, and the line under it says why.
- [ ] Start (or Skip) closes it. Quit and start again on the same library: it does not come back.
- [ ] On a fresh library, with the sheet up, press Command-Q (and, separately, choose Quit Inkwell
      from the menu-bar item): Inkwell quits at once. Start it again on the same library: the
      sheet shows again (quitting is not skipping).

## 2. Permissions (signed build; needs you)

- [ ] Settings > Permissions shows the four cards with their state now.
- [ ] "Hear the others" on a Mac that has never been asked: Allow shows macOS's system-audio
      prompt. Allow it, come back to Inkwell: the card reads Allowed within 5 s.
- [ ] **Revoking system audio turns its card red within 5 s.** With Inkwell's Settings open,
      turn Inkwell off in System Settings > Privacy & Security > Screen & System Audio Recording
      (the system-audio list). Click back into Inkwell: within 5 s the card turns seal red, says
      "System audio is off. Meetings record only your voice.", and Allow opens that pane.
      Record how many seconds it took: ______
- [ ] Turn it back on and come back: the card reads Allowed again.
- [ ] "Know your meetings": Allow shows the calendar prompt the first time; later it opens the
      Calendars pane. The card follows your answer when you come back.
- [ ] Leave Settings open and untouched for a minute with Activity Monitor on Inkwell: CPU stays at
      idle (the checks run when a screen with the cards opens, once at launch, and when you come
      back to the app, never on a timer).

## 3. Live (during the replayed meeting)

- [ ] "Live" appears in the sidebar under "While recording"; open it.
- [ ] Header: "Live meeting", the red dot with a running clock, the start time; legend
      "you / them / grey = still settling". The clock is the only thing ticking.
- [ ] What's being said: your lines (black dot, "You") and theirs (sepia, "Them"), each with its
      time into the meeting, in the order they were said. A grey italic line at the end is still
      settling and is replaced as the words settle; it is never kept (it is not in the Library
      afterwards).
- [ ] Your notes: type a line, press Return, type another, then click elsewhere. Each line is kept
      with the moment you started it (Library > the meeting, after S2.5: the notes lead the record,
      with their times).
- [ ] Edit an earlier line and move off it: the change is kept. Delete a line: it is gone from the
      record.
- [ ] Ask: Command-I puts the cursor in "Ask about this call". A question asked gets the honest
      "not available in this version" answer, never a made-up one. Far-end questions, when the
      replay has any, stack above it (newest first, at most four), and Command-1 to Command-4 ask
      them.
- [ ] When the replay ends, Live leaves the sidebar and the window shows Today.
- [ ] Resize the window as small as it goes and back: nothing overlaps, and the window never grows
      by itself as lines arrive.

## 4. Owed

- [ ] Promises from meetings, grouped by the person who owes them or by the meeting they were made
      in; a promise said twice is one row with "Said twice · merged".
- [ ] Late promises: seal-red ring and an "N days overdue" chip; others "Due tomorrow", "Due Fri",
      "Due 2 Oct" or "No date". The line under the title counts open, due this week and overdue.
- [ ] Clicking a promise's ring marks it done: it leaves the list and stays gone after a restart.

## 5. Settings

- [ ] Voice: the dictation key (fn: hold, speak, let go).
- [ ] Modes: each mode with its style and the apps it is for, each app by its name and icon. No
      bundle id (com.something.app) anywhere; an app that is not installed reads by its known name
      or "An app not on this Mac".
- [ ] AI: "Polish my words" reads On only while Apple Intelligence can polish. Turn Apple
      Intelligence off in System Settings, come back: the switch reads off, is greyed, and says
      why. Turn it back on (and wait for it to be ready): the switch is usable again and remembers
      your choice.
- [ ] Models: Dictation, Meeting transcript and Live words, each with the engine that serves it now
      and its measured accuracy. After a model finishes installing, the line changes to it without
      a restart.
- [ ] Meetings and Storage read true for this Mac; "Show in Finder" opens the library's folder.
- [ ] About: the version, update settings, the model credits (Parakeet under CC-BY 4.0 with its
      attribution), and every component's notice, each opening to its full licence text.
- [ ] About, last: "Rust libraries (N)" opens onto one row per crate, each opening to its licence
      text, which scrolls and can be selected. Opening and scrolling the whole list stays smooth.
      With VoiceOver, the disclosure is announced with its name and its state (collapsed or
      expanded), and each row with its crate, version and licence.

## 6. Design and accessibility

- [ ] Each screen side by side with its board on the design canvas (Live meeting, Owed, Settings):
      same order of parts, same words, same use of the seal red. Note any difference: ______
- [ ] Light and dark appearance: text stays readable everywhere, the sepia of "Them" included.
- [ ] VoiceOver (Command-F5): every control on these screens is announced with a name; Owed's
      rings say "Mark done:" and the promise; permission cards say their state; the order follows
      the screen from top to bottom.
- [ ] With Reduce Motion on, nothing on these screens animates.

## Finish

- Date, macOS version, commit: ______
- Signed off by: ______
