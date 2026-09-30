# Checklist: Live, Owed, Settings and the first run

About twenty minutes, by hand. What a script cannot check is here: permissions the system grants
(TCC), how the screens look and read against the design canvas, and VoiceOver. Run it after a change
to `mac/Sources/Inkwell/Screens/` and note the date, the macOS version and the commit.

Automated checks cover the logic behind each line (`swift test --package-path mac`): the
permission cards' reaction to a probe change, the Polish and the summaries-and-Ask toggles, modes
without bundle ids, owed grouping, the notes' saving, the ledger and the question stack, the model
downloads (only from a press, one at a time, failures and Retry), and every screen command against
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

- [ ] A fresh library opens the first-run sheet over the window: five steps shown as dots.
- [ ] Permissions step: four cards, "Hear you", "Hear the others", "Type for you", "Know your
      meetings", each with its state. Nothing is asked for until you press Allow.
- [ ] Models step (it downloads about 3 GB: only with your OK, into the scratch library's own
      models folder): each model not on this Mac with its licence, size and where it comes from
      (huggingface.co; Silero VAD from raw.githubusercontent.com), and the total. Nothing is fetched until you
      press Download (Activity Monitor > Network: Inkwell receives nothing before it). Then one
      model at a time, smallest first, with its bar, the others "Waiting". Continue works at once,
      and the downloads keep going through the rest of the first run and after it.
- [ ] Once Parakeet is in, Settings > Models reads Parakeet TDT v3 for Live words without a
      restart, and dictating shows live words in the Drop; once Qwen3-ASR is in, Dictation reads
      Qwen3-ASR 1.7B, and the first dictation after it is as quick as the next (it is loaded when
      its download ends, not by that take).
- [ ] A download that fails (turn Wi-Fi off while one runs): its row says "Couldn't download it"
      and why, in red, with Retry, and nothing tries again by itself. Wi-Fi back on, Retry: it
      downloads.
- [ ] Polish step: the switch is off and cannot be turned on if Apple Intelligence is off or not
      on this Mac, and the line under it says why. With Apple Intelligence on, switching it on
      asks first ("Turn on polish?", naming Apple's on-device model); Cancel leaves it off.
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
- [ ] Snippets: "No snippets yet." on a fresh library. Add one (trigger `my sig`, text `Kind
      regards`, category `Email`): it appears at once, with its category as a chip. Edit it (Edit,
      change the text, Save), switch it off and on, delete it: each change stays after you leave
      Settings and come back, and after a restart. Add stays greyed with a blank trigger. Text
      that looks like a link (`https://example.com`) reads as plain text: nothing is clickable.
- [ ] Voice commands: off on a fresh library, with the defaults listed ("scratch that · undo that
      …"). Those Inkwell does not do yet ("Undo the last dictation", "Pause or resume dictation")
      say "Not available in this version". Turn the switch on: the line reads "Say “inkwell”, then
      a command". Change the wake word (Return or Save), add a command (`sign off`, Type text,
      `Best, A. Writer`), switch one off, delete one: each stays after a restart.
- [ ] On a library made by the 0.2 import (`core/crates/ink-store/IMPORT-CHECKLIST.md`, section
      D): Snippets and Voice commands say "Brought over from Inkwell 0.2." until the first change.
      Voice shows the note about 0.2's key when it did not carry over (or when 0.2 started and
      stopped on separate presses); "Got it" removes it, and it does not come back after a restart.
- [ ] AI: "Polish my words" is off on a fresh library and turns on only through its dialog (see
      DICTATION-CHECKLIST section 3). Once on, it reads On only while Apple Intelligence can
      polish. Turn Apple Intelligence off in System Settings, come back: the switch reads off, is
      greyed, and says why. Turn it back on (and wait for it to be ready): the switch is usable
      again and remembers your choice (Apple's model again: no new dialog).
- [ ] AI: "Summaries and Ask" is off on a fresh library, and turns on only through its own dialog,
      which names where the transcript goes (see MEETINGS-CHECKLIST, Setup). Turning Polish on
      leaves it off, and the reverse. Turn it off: the switch reads off at once and stays off after
      a restart; turning it on again asks again.
- [ ] Models: Dictation, Meeting transcript and Live words, each with the engine that serves it now
      and its measured accuracy. After a model finishes installing, the line changes to it without
      a restart. Each model not on this Mac has Download (downloads: only with your OK); a second
      Download while one runs reads "Waiting" and starts when the first ends.
- [ ] Meetings and Storage read true for this Mac; "Show in Finder" opens the library's folder.
- [ ] About: the version, update settings, the model credits (Parakeet under CC-BY 4.0 with its
      attribution), and every component's notice, each opening to its full licence text.
- [ ] About, last: "Rust libraries (N)" opens onto a list of its own that scrolls, one row per
      crate, each opening to its licence text, which scrolls and can be selected. Opening the list
      and scrolling it to the end stays smooth, and the Settings page does not grow by its length.
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
