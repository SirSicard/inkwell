# Mac capture checklist (S2.1a)

Capture needs **Microphone** and **System Audio Recording** for the terminal app you run it from,
which CI and an agent's shell do not hold. The unit tests cover every decision (routing, the
Apple-daemon rule, zero detection, the tone analysis, process-object mapping, the IOProcs under
`assert_no_alloc`); this list covers what only a real session shows.

Run `scripts/mac-tcc-checklist.sh` from the repository root. It builds `examples/capture_check.rs`
and walks through steps 1 to 5, pausing where you need to act. The binary prints levels, counts and
verdicts only, never audio. Every capture step also prints an I4 line (allocations in real IOProc
callbacks), which must be `PASS I4: 0 allocations`.

## Setup

In System Settings > Privacy & Security, allow your terminal app (Terminal, iTerm) under
**Microphone** and under **Screen & System Audio Recording > System Audio Recording Only**. Quit and
reopen the terminal after changing either.

## Lines

| # | Step | Must see |
|---|---|---|
| 1 | Devices, speakers as output | `route: record <built-in mic> ... because DefaultInput` |
| 2 | Permissions and the tone probe | `Microphone: Granted`; `system audio probe: Heard -> Granted` |
| 3 | 10 s with nothing playing | `Far: Idle` then `PASS Far: Idle`: no callbacks while nothing plays is idle, not broken |
| 4 | 60 s Zoom test call (zoom.us/test): speak when it asks, leave before the 60 s end | `PASS Mic: Signal` and `PASS Far: Signal`; measured rates within 1 % of declared; `detect: + us.zoom.xos` during the call, `detect: - us.zoom.xos` after you leave, no `com.apple.` lines |
| 5 | System audio revoked (the terminal switched off under System Audio Recording Only; if it still reads Granted, reopen the terminal and run `capture_check --permissions --probe`) | `system audio probe: Silence -> Denied`. You may hear a faint 0.4 s tone. Switch it back on after. |

With Bluetooth earbuds (optional, run by hand):

| # | Command | Must see |
|---|---|---|
| 6 | `capture_check --devices` with the earbuds as output | `because BuiltInForBluetoothOutput`: the built-in mic, not the earbuds |
| 7 | `capture_check --capture 20 --headset-mic`, speak, then stay silent | `Mic: Signal` at 16000 Hz, a high `zeros` share in the silent windows (the headset gates to zeros): the headset mic works outside an aggregate |
| 8 | `capture_check --capture 20 --app us.zoom.xos` during a Zoom call | `tapping us.zoom.xos (N processes)` with N ≥ 1, and `PASS Far: Signal`: an app tap by process object, not pid |

## Record

Green when lines 1 to 5 pass. Record anything else it printed that looks wrong: a non-zero
`discontinuities`, `skipped`, `untimed` or `overruns` count; a measured rate far from the declared
one; `detector: N read errors` with N above 0; and the probe's time (it should take about a
second). If line 2 shows `NoAudio` or `NotTheTone`, record the `tap callbacks` and `output
callbacks` line under it.
