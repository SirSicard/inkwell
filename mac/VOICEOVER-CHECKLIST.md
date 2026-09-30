# VoiceOver checklist: the shell

About five minutes, by hand: VoiceOver cannot be driven from a script. Every sidebar item must be
reachable and must work with VoiceOver alone. Run it after any change to the sidebar, the window
or the menu-bar item, and note the date, the macOS version and the commit.

VO means Control-Option (or Caps Lock, if VoiceOver uses it as its modifier).

## Setup

- [ ] Build and open the app: `mac/scripts/build-mac.sh && open mac/build/Inkwell.app`
      (an ad-hoc build is fine: VoiceOver needs no permission from the app).
- [ ] Turn VoiceOver on: Command-F5.

## The menu-bar item

- [ ] VO-M, then VO-M again, moves into the menu extras. Move to Inkwell: VoiceOver says
      "Inkwell" (not "button" or "image").
- [ ] VO-Space opens its menu. VoiceOver reads the core's status line (for example
      "Ready (core …)"), then "Open Inkwell", "Open at Login" with its checked state, and
      "Quit Inkwell".
- [ ] "Open Inkwell" with VO-Space opens the main window, and VoiceOver moves into it.

## The window and the sidebar

- [ ] VoiceOver names the window "Inkwell" and the screen title ("Today").
- [ ] Move to the sidebar (VO-Left/Right, or Tab). VoiceOver announces the list ("Sections") and
      its rows by name with their position: Today, Library, Owed, Settings.
- [ ] On each row in turn, press VO-Space. The content changes, and VO-Right into the content
      reads that row's name as a heading. Every row: Today, Library, Owed, Settings.
- [ ] The arrow keys move through the rows with the list focused, and the content follows.
- [ ] The ink beside the content (the wide zone with the wordmark on Today, the narrow rail
      elsewhere) is never announced: it is decorative.
- [ ] VO-U (the rotor) lists the screen title under Headings.
- [ ] Live is not listed: it appears only while a meeting runs, and is checked with the Live
      screen (SCREENS-B-CHECKLIST.md, sections 3 and 6).

## Closing and quitting

- [ ] Command-W closes the window, and the Dock icon goes.
- [ ] The menu-bar item's "Open Inkwell" brings the window back, on the screen last shown.
- [ ] "Quit Inkwell" in that menu quits the app.

## Result

Date, macOS version, commit:

Failures (the step, and what VoiceOver said instead):
