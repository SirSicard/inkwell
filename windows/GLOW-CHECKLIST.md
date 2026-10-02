# Glow on Windows: the checks that need a desktop session

The Windows shell's Glow redesign was compiled (`dotnet build Inkwell.slnx -c Release -p:Platform=x64`),
not run: no test suite, no dry run. Everything below needs a signed-in desktop, a microphone and,
for some items, the installed app. Tick each with what you saw.

## Look and mode

- [ ] The window opens with no Mica: the mode's background, the orb behind the content (upper
      right of centre), the sidebar and the screens drawn over it.
- [ ] Settings > Appearance > Light, Dark and Match system each switch the whole window at once,
      the caption buttons (minimise, maximise, close) included; Match system follows Windows'
      app mode when it changes.
- [ ] Each mode keeps its own dot preset: pick one in Light, switch to Dark, pick another, switch
      back; the first is still chosen.
- [ ] Your colour and theirs: the swatch opens a picker; the colour is set when the picker closes;
      "Use the preset's" goes back. A very dark colour at night, or a very pale one by day, is
      shown adjusted (lighter, or deeper).
- [ ] Cards are translucent over the orb and blur it (in-app acrylic over the orb's
      SwapChainPanel). With Windows' Transparency effects off they turn solid.
- [ ] The faces: Sitka Display for the greeting and titles, Sitka Text for the record's text,
      Segoe UI Variable for the interface, Cascadia Mono for times. None falls back to another face.

## The orb, the edge glow and idle

- [ ] At rest the orb is one still frame; with the window open and nothing live, the ink's frame
      count does not move and the GPU is idle.
- [ ] Hold the dictation key: the orb wakes in your colour and the window's edge glows in it,
      answering your voice; let go and both settle.
- [ ] Record a call: the edge carries both colours, leaning away from whoever speaks; Stop, and
      the edge goes out while the orb blots down to the ink drop.
- [ ] Settings > Appearance: "Glow the window's edge" off, no edge; "Always still", one still frame
      per change for the orb and the edge, and the Live dot does not pulse.
- [ ] Windows' Animation effects off: the same as Always still.
- [ ] High Contrast on: solid cards, borders in the text colour, the orb dimmed behind the text.

## Window and keys

- [ ] The sidebar's header says Inkwell; there is no rail. Owed shows the overdue count; Live,
      while recording, shows a dot in their colour that pulses.
- [ ] The title bar says "Inkwell · {route}", and the core's state only while it is not ready.
- [ ] Ctrl+1 to Ctrl+4 open Today, Library, Owed and Live (Live only while recording); Ctrl+,
      opens Settings; Ctrl+F puts the cursor in the search; Ctrl+Shift+R records; Ctrl+. stops.
      On Live, Ctrl+1 to Ctrl+4 answer the asks and Ctrl+. stops, as before.
- [ ] The title bar's "…" holds File (Record Now or Stop Recording) and View (the routes,
      Settings, Find, Appearance), each with its keys; Appearance's three choices work.

## Screens

- [ ] Today: the date, "Good morning/afternoon/evening" large, the listening and dictation-key
      line, Record now; while recording, the live card with Open and Stop and the running time;
      after Stop, the card says how far the final pass has come (transcribed, speakers sorted,
      summarized) until the record is written.
- [ ] Live: the large Stop; the legend's and the lines' dots in the theme's colours.
- [ ] Owed: Mark done shows the Undo toast for a few seconds, and Undo brings the promise back;
      "▸ Partner call at 38:52" plays the record from there.
- [ ] Library: opens on All; the kind chips narrow it; pressing All widens it again.
- [ ] Record: the speaker dots and the waveform's lanes in the theme's colours (text stays in the
      text colour); Space plays and pauses unless the cursor is in a text field.

## The Drop

- [ ] The pill follows the mode (night or day), with the orb in its circle on the left, the state
      above and the line in Sitka beside it.
- [ ] The offer to record a call: the pill grows, its two pill buttons answer, and the Drop never
      takes focus from the app you are in.
- [ ] If the Drop's ink cannot draw (the plain GDI panel shows), the panel is in the mode's colours.

## Tray

- [ ] The tray icon's dot: none at rest, your colour while dictating, theirs while recording, the
      alert colour when the far end is silent; a new preset redraws it.
- [ ] Right-click: the status line ("Ready", "Recording · {title} · 12:04"), Record Now or Stop
      Recording, Dictation (ticked while on), Open Inkwell, Settings…, Start with Windows,
      Check for Updates…, Quit Inkwell. Left-click opens the window.

## Start with Windows and updates (the installed app)

- [ ] In a copy run from a folder, Start with Windows is off and greyed, and General says only the
      installed app can.
- [ ] In the installed app, turning it on adds an "Inkwell" value under the user's Run key naming
      the launcher in the install folder; signing in again starts Inkwell in the notification
      area; turning it off removes the value.
- [ ] General > "Check for updates automatically": off by default. On, the next launch checks once
      and the row says what it found; nothing checks again until the next launch or Check Now.
- [ ] Tray > Check for Updates… opens Settings and checks.

## Settings and the first run

- [ ] Settings lists General, Appearance, Permissions, Dictation, Modes, Snippets, Voice commands,
      AI, Meetings, Models, Storage, About, with Settings at the sidebar's foot; Inkwell 0.2's
      import is in General.
- [ ] AI > Local only: on by default; off lets a chosen provider be called; on again refuses it
      ("Nothing leaves this computer"); choosing a provider off this PC turns it off, as before.
- [ ] First run (reset onboarding.done): Welcome's orb plays dictating then a call, and its title
      is the size of the other steps'; Appearance comes after Models (and after the 0.2 import when
      offered); Ready's box takes a dictation and its orb answers the voice.
