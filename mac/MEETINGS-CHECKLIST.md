# Checklist: meetings end to end, and the dogfood week

What a script cannot check: real calls, the permissions macOS grants (TCC), what the Drop and the
screens say while a call runs, and a week of real use. Section 1 takes about twenty minutes; the
dogfood week (section 6) is five real meetings. Note the date, the macOS version and the commit.

Automated checks cover the logic behind each line: `cargo test --workspace` in `core/` (detection's
rules, meeting start and stop, Ask, the far end's bands, retention's secure delete, the `kill -9`
recovery with a real child process, replay determinism, the Bluetooth-mic flag, summary items,
recipients, looks-done, no summary or Ask without the `meetings` consent) and
`swift test --package-path mac` (the consent Drop and its answers, the summaries-and-Ask switch, the
watchdog's warning and the probe's reaction, Live, the ledger's window, Owed, cited decisions, the
meeting settings, Foundation Models' structured answers).

Keep what you write down here to counts, times and yes/no. Never paste a transcript, a summary, a
name or a meeting title into this file or a bug: the repository is public.

## Setup

- [ ] Build the signed app with the engines (an ad-hoc signature is a new app to TCC every time):
      `INK_SIGN_IDENTITY=... NEMO_SPEECH_DIR=... mac/scripts/build-mac.sh --engines`.
- [ ] Models installed (Settings > Models lists Qwen3-ASR, Parakeet, Silero and Nemotron as "On
      this Mac"; Parakeet gives the live words), and
      Apple Intelligence on (Settings > AI: Polish can be turned on). Without Apple Intelligence
      meetings get no summary, and say so.
- [ ] Settings > AI, before turning anything on: "Summaries and Ask" is off, and says meetings are
      recorded and transcribed with no summary. Record a short call: it finishes normally, its
      record's Summary tab says "Summaries are off until you allow them in Settings > AI", and Ask
      during it answers that it needs your OK in Settings > AI.
- [ ] Turn "Summaries and Ask" on: a dialog asks first ("Turn on summaries and Ask?"), says the
      meeting's transcript goes to a language model and names it (Apple's on-device model: "the
      transcript stays on this Mac"). Cancel leaves it off; "Turn On Summaries and Ask" turns it on.
      Polish stays as it was (each has its own consent).
- [ ] Permissions: Settings > Permissions shows "Hear you" and "Hear the others" allowed.

## 1. A call, from the offer to the record

- [ ] Today, at the foot of the ink: "Listening for meetings" and "Record now".
- [ ] Start a call in a meeting app (a test call with yourself on a second device is enough).
      Within about 5 s of the app opening the microphone, the Drop appears at the bottom of the
      screen: "<App> opened the microphone", "Recording keeps both sides on this Mac. Tell the
      others you are recording.", and two buttons. Nothing is recorded yet (no red dot anywhere).
- [ ] While the Drop asks, keep typing in another app: every key lands there. Click "Not this
      one" without leaving that app: the Drop goes and the app you were in stays in front.
- [ ] Hang up and start another call: the Drop asks again. Click "Record this call": the Drop turns
      to "● REC · <App>" with the latest line said, and the sidebar gets "Live".
- [ ] Live: the title is the calendar event going on now (when Calendar is allowed and the call
      is on it), otherwise "<App> call"; the app, the start time; your lines and theirs as they
      settle; your drop pulses when you talk, theirs when they do (the far end now has bands of its
      own).
- [ ] With Bluetooth headphones: the header says which mic is used and why ("..., because your
      headphones are Bluetooth").
- [ ] Ask "What have we decided so far?" (⌘I): an answer in plain words within about 10 s, or,
      without Apple Intelligence, "Answers need Apple Intelligence, which is off or not ready on
      this Mac." Links in an answer are never clickable.
- [ ] Hang up. Within about 15 s the Drop turns to "Blotting · final pass" (the meeting ends when
      the app lets go of the microphone), then goes. (Or press Stop in Live.)
- [ ] Today shows it as the last meeting with its summary's first lines; the record opens with the
      final transcript, the summary, "Decided, and where" with each decision's line, and what you
      promised under Owed, grouped "To <person>" when the call said who it was for.
- [ ] Record now (Today) records without a call: stop it from Live.

## 2. The watchdog: revoking system audio mid-call (needs you; the Verify line)

- [ ] During a recorded call where the other side is talking, turn Inkwell off in System Settings >
      Privacy & Security > Screen & System Audio Recording (the system-audio list).
- [ ] **The warning appears within 20 s**: the Drop's border turns seal red, "The other side is
      silent", "System audio is off, so only your voice is being recorded.", and "Allow system
      audio". Today's banner says the same. Seconds from the switch to the warning: ______
- [ ] Click back into Inkwell (it checks the permission when it becomes active): the Drop says
      "System audio is off" at once, even before the silence is long enough for the watchdog.
- [ ] Turn it back on: the warning clears once their audio arrives again.

## 3. Crash recovery (needs you)

- [ ] During a recorded call, kill Inkwell outright: `kill -9 $(pgrep -x Inkwell)`.
- [ ] Start Inkwell again. Within a minute, Today says "A meeting was finished after Inkwell quit
      unexpectedly", and the meeting is in the Library with a final transcript up to the kill.
      Play its last minute: the audio runs to within a second or so of the kill (the core's own
      test measured 0.03 s lost; the limit is one 10 s chunk).

## 4. Settings

- [ ] Settings > Meetings: turn "Offer to record calls" off; Today reads "Not listening for
      meetings" and a call gets no Drop. Turn it on again.
- [ ] "Use the headset's microphone" on, with Bluetooth headphones: the next meeting's header names
      the headset's mic. Turn it off again.
- [ ] Settings > Storage > Keep records: choose 30 days on a library that has meetings or
      dictations older than that (or wait): they leave the Library at once, imports stay, and
      Settings > Storage's sizes go down after a relaunch. Set it back to Forever.
      Nothing in the repository makes records older than 30 days in a scratch library
      (`seed_library`'s are at most about four days old), so the deletion side can't be checked by
      hand without deleting real records; `cargo test -p ink-ffi --test meetings` covers it
      (`retention_deletes_old_records_whole_and_leaves_no_trace_of_their_words`,
      `a_sweep_keeps_what_an_import_brought_in`), and `--test import02` covers imports
      (`imported_dictations_outlive_a_retention_change`).

## 5. Consent and honesty (read, don't just click)

- [ ] Nothing in the Drop, Live, Settings or Today says Inkwell is invisible, undetectable, hidden
      or secret. The Drop shows whenever a meeting records, on every Space and beside full-screen
      apps.
- [ ] Inkwell never starts recording without a click (the offer is only an offer).
- [ ] Record now: Live says it records everything this Mac plays. A call app Inkwell can't hear
      alone (if you meet one): the Drop says so until the first line, and Live says so throughout.

## 6. The dogfood week: five real meetings (the plan's exit is Mattias's private dogfood file)

For each meeting, note in your private dogfood file (not here):

- [ ] the app, the length, headphones or speakers, whether both sides were recorded;
- [ ] the Drop's offer came, and within how long; the meeting ended on its own when the call did;
- [ ] from hang-up to the record being final (Blotting gone): ______ s for ______ min;
- [ ] the summary's headline fits; decisions show their line; Owed has what you promised, to the
      right person; "Looks done" suggestions, if any, were right or wrong;
- [ ] any warning shown, and whether it was true;
- [ ] the live ledger's size, from the log after the meeting (counts and bytes, never words):
      `log show --last 1d --predicate 'subsystem == "com.inkwell.app" AND category == "core"' | grep "meeting ledger"`
      The shell keeps at most 500 lines of a meeting in memory; the line says how many it saw,
      let go of, and the most bytes it held.
- [ ] the ink: in a quiet room your drop stays nearly still; your voice throws droplets; the far
      end's drop pulses with their voice (the level map was tuned on the AMI fixture: note if it
      sits too high or too low on your mic or your calls).

Open P0 bugs at the end of the week: ______ (the plan's exit needs none).
