# Modes checklist: editing modes in Settings, on the desktop

What an SSH session cannot check: the editor dialog, its pickers and menus, Narrator, and how the
rows look in a narrow window. About twenty minutes at the PC, signed in, with a scratch library
(`INK_DATA_DIR`). Note the date, the Windows build and the commit under Result.

Automated checks cover the logic behind each line: `ModesEditorTests` (every refusal's words, the
save, consent and confirm flows, the editor against the real core), `PolishConsentsTests`,
`ModesLayoutTests` (the rows and the editor at the window's 720-epx minimum) and `DropModelTests`.

## Rows

- [ ] Settings > Modes at 720 epx wide: each row's name above its chips, apps, note and buttons;
      at the default width, the name beside them. Nothing is cut or runs past the card.
- [ ] With no language model, a mode with polish on shows no "Polish" chip. Choose a provider in
      AI, then turn Polish my words off: the chip reads "Polish · off", quieter, its tooltip
      "Polish my words is off in AI".
- [ ] Narrator on a row: the mode's name, then its chips ("Polish, off: …"), apps and each button
      by name ("Edit Chat", "Delete Chat").

## The editor

- [ ] Add a mode: the dialog opens with the name focused; Enter saves, Esc cancels and saves nothing.
- [ ] A blank name, "Casual", and a name another mode has each say why under the title, and
      Narrator reads it as it appears.
- [ ] Renaming a mode says "Voice commands use the new name."
- [ ] Polish instructions: the default shows as the placeholder; typing shows "n / 2,000";
      Use the default clears it. Line breaks are kept as typed.
- [ ] Add an app > Running now lists apps with a window (not Inkwell, not ApplicationFrameHost); one
      in another mode reads "Slack — in Chat", and picking it says "Moves Slack from Chat."
- [ ] Add an app > Browse… opens a file picker on .exe files; an app chosen there is listed by name
      and icon. Remove (−) takes it out.
- [ ] The default mode ("Everywhere else") can be renamed but has no apps and no Delete.

## Models and consent

- [ ] With an own-key provider chosen and no OK for it, pick it as a mode's model and Save: a card
      inside the dialog asks "Polish “Chat” with Groq · …?", focus on Cancel, Save waits. Cancel
      saves nothing; Send to Groq records the OK and saves.
- [ ] Settings > AI lists "Polish may send to" with each destination and Revoke; Revoke takes one
      away, and the last turns polish off.
- [ ] Choose another provider in AI: a mode on the first one says its model isn't available and
      shows no chip; a dictation in that mode's app shows "Not polished" on the Drop.

## Delete

- [ ] Delete asks "Delete “Chat”?", naming where its apps go; Cancel is the default.

## Result

- Date, Windows build, commit:
- Notes:
