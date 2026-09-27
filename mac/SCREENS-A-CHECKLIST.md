# Screens A checklist: Today, Library, a record

About twenty minutes, by hand, against the design canvas (the "Today" and "Library · a meeting
record" artboards). Each line is an element of the canvas: tick it when the app shows it and it
behaves as written, or write what differs under **Result**. Some lines need a person because they
need a permission (the calendar, system audio) or ears (playback); those say so.

## Setup

- [ ] Quit every other Inkwell 1.0 build (one copy runs at a time; a second only brings the first
      one's window forward).
- [ ] Seed a scratch library. Every word in it is synthetic; the audio is the AMI fixture,
      replayed through the meeting chain. Set the offset to your time zone's, in minutes, so
      "by Friday" lands on Friday:
      `cd core && SEED_UTC_OFFSET_MINUTES=120 cargo run -p ink-ffi --example seed_library -- /tmp/inkwell-screens`
- [ ] A second library whose newest meeting kept only your voice, for the banner:
      `SEED_UTC_OFFSET_MINUTES=120 SEED_FAR_SILENT=1 cargo run -p ink-ffi --example seed_library -- /tmp/inkwell-screens-deaf`
- [ ] Build and run on the first: `mac/scripts/build-mac.sh` (signed with your identity, so the
      calendar and audio permissions stick), then
      `INK_DATA_DIR=/tmp/inkwell-screens mac/build/Inkwell.app/Contents/MacOS/Inkwell`.
- [ ] Make the window 1280 × 800 (the canvas's size) for the comparisons.

## Today

- [ ] The date in small spaced capitals, then a greeting for the hour ("Good afternoon").
- [ ] Last meeting: its title in New York is the summary's headline ("Launch moves to the 14th,
      after the design review"), not "Untitled" or a date.
- [ ] Its line reads "Today 13:34 · 1 min · Zoom · Alex, Robin, Sam" (your times).
- [ ] Below, the summary's first paragraph, rendered: **another week** bold, *fourteenth* italic,
      no `**` or `*` anywhere.
- [ ] It is the meeting that **started** last. (The Standup is older, and the pricing call was
      yesterday; neither shows here.)
- [ ] "Open record" opens it in the Library. "Play from 00:02" opens it and plays from the first
      promise (needs ears: you hear the meeting from 2 s).
- [ ] Up next, calendar never asked: "See your next meeting here." and a "Show my next meeting"
      button. Nothing asks for the calendar until you click it.
- [ ] **Calendar, grant (TCC; needs you):** click "Show my next meeting". macOS asks for calendar
      access with Inkwell's text ("Inkwell reads your calendar to show your next meeting…").
      Allow. Up next shows your next timed event today: its title, "15:30 · Zoom · in 42 min"
      (the app comes from the event's link, place or notes), and "Records when Zoom opens the
      microphone". With nothing in the next 12 hours: "Nothing else on your calendar today."
- [ ] **Calendar, change:** add an event 30 minutes from now in Calendar.app. Up next shows it
      within seconds, without leaving Today.
- [ ] **Calendar, the minute:** leave Today open for two minutes: "in N min" counts down. Cover
      the window, or minimise it, for two minutes and come back: it shows the right minute again
      at once (it did not redraw while hidden).
- [ ] Settings' "Know your meetings" card turns green after the grant (it is the same permission).
- [ ] **Calendar, deny (TCC; needs you):** `tccutil reset Calendar com.inkwell.app`, relaunch,
      click "Show my next meeting", and choose Don't Allow. Up next reads "Calendar access is off,
      so Inkwell can't show your next meeting." with "Open Calendar settings", which opens System
      Settings at Privacy & Security > Calendars. Turn it on there and come back: Up next shows the
      event (Today re-reads when the app comes to the front), and so does the Settings card.
- [ ] Owed soon: the first three of the Owed screen's list, soonest first, each with a circle, its
      text, and a line with the due day ("Due Fri"), the record's title and "▸ 00:02". An overdue
      one is in the alert colour, "2 days overdue". "All 3" (the Owed screen's count) opens Owed.
- [ ] The circle marks one done: it leaves the list, the next one moves up, and "All" counts one
      fewer.
- [ ] "▸ 00:02" opens its record and plays from where it was said.
- [ ] The foot: "Dictated today · 38 words · under 1 min" and "This week · 3 meetings · 2 min".
- [ ] The toolbar's "Search everything said": typing opens the Library's matches.
- [ ] **Needs you (the watchdog and the library):** run on `/tmp/inkwell-screens-deaf`. A card with
      a red mark: "Inkwell didn't hear the other side of your calls", "Your last meeting kept only
      your own voice…", and "Allow system audio", the same request as Settings' "Hear the others"
      card: macOS's prompt the first time, System Settings at its pane after that.
- [ ] **Needs you, system audio denied (TCC; needs you):** with a meeting recorded and system
      audio allowed once (so the app has asked), turn Inkwell's system audio off in System
      Settings and come back to Today (it checks when it appears and when the app comes back to
      the front while it shows; the check plays a muted tone). The card reads "Inkwell can't hear the other side of your
      calls" with "System audio has been off since <date>, so your meetings kept only your own
      voice." once a meeting has kept only your side. (Until the app has asked for system audio,
      the check cannot tell and says nothing: onboarding asks.)
- [ ] **Needs you, microphone denied (TCC; needs you):** turn the microphone off for Inkwell and
      come back to Today: "Inkwell can't hear you" with "Allow the microphone". Turn it back on
      through that button's pane and come back: the card goes.
- [ ] Two or more needs: the most urgent shows, with "N more" to show the rest.
- [ ] When Today's counts cannot be read, the banner says "Inkwell couldn't check the other side
      of your calls" with "Try again", and the foot says "couldn't be counted"; neither reads as
      zero. (Unit-tested; to see it by hand, the library must fail to answer a query.)

- [ ] The ink zone beside Today shows the ink with INKWELL knocked out of it, and the Library
      the narrow rail with ink (S2.4's ink; checked here because this step's screenshots were
      taken with the screen locked, when no window is on screen and the ink rightly draws
      nothing: its layer is plain paper until the first visible frame).

Not in this step, by design (so not a failure here):
- The ink zone's own lines ("Listening for meetings", "Hold fn to dictate") and "Record now":
  meeting capture from devices is S2.8. The zone and the wordmark are S2.4's.
- The Owed count beside the sidebar row: S2.6 owns the sidebar's Owed.

## Library

- [ ] The list column: "Library" with the count, and the chips Meetings, Dictations, Files, with
      Meetings pressed.
- [ ] Meetings list newest first: the launch meeting (today), the pricing call (yesterday), the
      Standup (Wed 23 Sep). Each row: the title (the summary's headline), then "Today · 13:34 ·
      1 min".
- [ ] Dictations: three rows, each titled by its first words (no "Untitled"), newest first.
- [ ] Files: one, "IS1009a-mic", titled by its file's name.
- [ ] Pressing the pressed chip again shows every kind, still newest first.
- [ ] Arrow keys move through the list, and the record follows.
- [ ] Search "budget": two matches from the launch meeting, each with its words and "Today ·
      00:10". A match opens its record with the playhead at that moment.
- [ ] Search "zebra": "Nothing said matches “zebra”."
- [ ] An empty library (run on an empty directory): "No meetings yet" and a sentence. No command
      line, no file path, no "run …" anywhere.

## A record

- [ ] The title in New York, "Today 13:34 · 1 min · Zoom · You, Alex, Robin, Sam", and three
      buttons: Copy summary, Share, More.
- [ ] Copy summary puts the rendered summary on the clipboard: paste it in TextEdit and find no
      `#`, `**` or `- `.
- [ ] Share offers the system's share sheet with the title and the summary. More offers Copy
      transcript.
- [ ] Tabs: "Notes and transcript", "Summary", "Owed · 1", underlined like the canvas's.
- [ ] Notes and transcript, left: "Your notes, filled in": each note you typed (three here) with
      its chip, then what was being said as you wrote it (who, in their colour, and what), then
      what was promised around then (a check mark). Chips are small bordered times in sepia.
- [ ] Right: "What was said" and "Blotted 13:35 · final" with a drop. Each line: its time, a dot
      (ink for you, sepia for them), who (You, or the name; unnamed speakers read "Speaker 1"),
      and the words in New York.
- [ ] Clicking a chip or a line plays from there (needs ears), the playhead jumps to it, and that
      line is shaded, as the canvas's 12:41 line is.
- [ ] Summary: the headline, then "Launch date", "Beta", Decisions, Actions, Open questions as
      headings; bullets and "1." "2." as list markers; bold and italic as type. No markdown
      token anywhere on the tab.
- [ ] A link in a summary shows as plain words: not coloured, not clickable, and nothing opens
      (the summary is written from what the other side said, so its links are not trusted).
- [ ] Below the summary, "Where it was said": each promise with the line it came from and its
      chip. (Decisions have no line yet: see the report.)
- [ ] Owed: each promise with a circle (done / not done), its owner and due, and its chip.
      Marking it done strikes it through and updates Today's Owed soon.

## The player

- [ ] At the foot: a round play button, a two-lane waveform (them above in sepia, you below in
      ink; the played part solid, the rest faded), the red playhead, "00:02 / 00:30", and You and
      Them sliders.
- [ ] The play button plays and pauses (needs ears). Both sides play in step: the far side's
      words land when the transcript says.
- [ ] Clicking or dragging on the waveform moves the playhead there.
- [ ] Them at zero leaves only your side; You at zero only theirs.
- [ ] Playing to the end stops at "00:30 / 00:30"; Play again starts from the beginning.
- [ ] With no output device (or it disconnects), the bar says "This recording can't be played
      right now." instead of failing silently.
- [ ] Idle, the Library draws nothing: Activity Monitor shows Inkwell at 0 % CPU with a record
      open and paused, and no audio engine runs until Play.

## Accessibility

- [ ] VoiceOver reads each chip as "Play from 00:02", each line as "00:02, You: …", the waveform
      as "Waveform: you in ink, them in sepia" with the time as its value; VO-Up/Down on it moves
      10 s.
- [ ] Every button reads its name (Copy summary, Share, More, Mark done: …, Play/Pause, Your
      volume, Their volume, the filter chips with their selected state).
- [ ] Section labels (Last meeting, Up next, Owed soon, What was said, …) are headings in the
      rotor.
- [ ] Reduce Motion on: the playhead moves once a second instead of smoothly; nothing else moves.
- [ ] Dark mode: night paper, and every text readable (sepia names turn lighter).
- [ ] The VoiceOver checklist's "reads that row's name as a heading" now finds the screen's own
      first heading: the date and greeting on Today, "Library" in the Library.

## Result

Date, macOS version, commit:

Failures (the line, and what the app did instead):
