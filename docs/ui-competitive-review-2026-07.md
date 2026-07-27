# Inkwell UI review against the 2026 field

Research pass over the competing dictation apps, measured against Inkwell's
actual rendered UI rather than against memory. Numbers below were computed from
the running app, not estimated.

---

## 1. What the category converged on

Every serious competitor now shares the same three-part shape, and Inkwell has
two of the three.

| | Menu-bar resident | Floating HUD while recording | Main window is secondary |
|---|---|---|---|
| Wispr Flow | yes, "invisible until needed" | pill, bottom-centre, **live text** | yes |
| superwhisper | yes | mini pill, live waveform | yes |
| VoiceInk | yes | overlay | yes |
| Handy | yes (tray) | overlay, position configurable | yes |
| **Inkwell** | **tray, but the app is Regular until you close the window** | **97px ink blob, fixed bottom-centre, shows nothing** | **no, the window is the app** |

Three specific things the field does that Inkwell does not:

**The HUD carries information.** Wispr shows the transcription forming inside
the pill; superwhisper shows a live waveform. Inkwell's overlay is a 97px ink
blob whose "soundwave" bars are a sine function of time, not audio. It is the
surface a user sees on every single dictation and it is the one surface that
tells them nothing. Worse, it lies: it animates identically whether the mic is
picking up your voice or nothing at all, which is exactly the failure that hid
the -65 dBFS capture bug for as long as it did.

**The HUD is movable.** Wispr hardcoded the pill to bottom-centre and someone
shipped a whole third-party utility, PillFloat, purely to move it. That is a
loud signal about a small feature. Inkwell hardcodes bottom-centre with an 80px
margin.

**Modes, not settings.** superwhisper's differentiator is Modes: a named bundle
of model + formatting + prompt that auto-activates per app, each with its own
shortcut. Inkwell has the same raw material scattered across three separate
panels (per-app styles, AI polish prompt, model selection) with no concept
tying them together. This is the single biggest structural gap, and it is
mostly a UI idea rather than new engine work.

---

## 2. What Inkwell is doing wrong (measured)

### 2.1 The tertiary text colour fails accessibility, in 118 places

`--color-text-tertiary` is `rgba(255,255,255,0.35)`. Composited on the base
background it is **3.17:1**, against a WCAG AA floor of 4.5:1 for body text.

It is used **118 times**, and it is the colour of nearly every description line
under a setting. It also lands mostly on 10 and 11px type, so it fails on size
and contrast at once.

Measured, on `#0e0e11`:

| Token | Alpha | Contrast | |
|---|---|---|---|
| primary | 0.95 | 17.37:1 | fine |
| secondary | 0.60 | 7.29:1 | fine, AAA |
| **tertiary** | **0.35** | **3.17:1** | **fails AA** |
| accent `#c8956c` | solid | 7.32:1 | fine |

The fix is one token. `0.35 -> 0.55` gives roughly 6:1 and changes nothing else.
This is the highest value-per-character change in the whole review.

### 2.2 There is no type scale

65 arbitrary sizes across the app, no ramp:

```
26x  text-[11px]
17x  text-[10px]
 9x  text-[13px]
 7x  text-[15px]
 5x  text-[12px]
 1x  text-[9px]
```

A 9px label exists. The dominant body size is 11px. On a Retina Mac in 2026 this
reads as a cramped web app, not a native tool, and it is the single biggest
reason the app looks less premium than superwhisper in a screenshot. Competitors
sit on the system 13px body with 11px reserved for genuine metadata.

Proposed ramp, four steps, nothing else allowed:

```
11px  metadata only (mono, uppercase labels, counts)
13px  body and controls        <- new default, was 11
15px  section headings
20px  view titles
```

### 2.3 The ink panel is 35% of the window, permanently

Measured at a 1280px viewport: the ink column is exactly 35% of the app, on
every view, forever. On the Dashboard with no transcripts, the app is a third
giant black splatter and two thirds empty space.

The blob is also tuned wrong at that size. The app uses `warpIntensity = 0.20`,
which was set for the 97px overlay where it is a few pixels of wobble. Blown up
to a 450px column the boundary swings ~79% of its own radius, which is why it
reads as a Rorschach rather than ink. I already fixed exactly this on the
homepage by halving the warp; the app never got the same treatment.

The identity is worth keeping. Spending a third of the window on it at all times
is not.

### 2.4 Dark only, in the Liquid Glass era

Inkwell has no light mode. macOS 26 shipped Liquid Glass, where system sidebars
and toolbars tint from the content behind them and follow the user's accent
colour. A flat, dark-only, opaque-surface app on Tahoe does not look
deliberately different, it looks like it was not updated.

The interesting part: **Inkwell already owns a light palette.** The ink panel's
cream (`#f0ede8` field, `#0a0a0a` ink) is a complete light theme sitting inside
the app doing decorative duty. A light mode is an inversion of assets that
already exist, not a new palette to invent.

---

## 3. What is missing

Ranked by value against effort.

1. **A real HUD.** Live level from the Rust amplitude events that already exist,
   plus the transcript appearing when it lands. This is the most-seen surface
   and currently the least informative one. The events are already emitted; this
   is frontend work only.
2. **Modes.** Bundle model + style + polish prompt + per-app activation under one
   named object. Every component exists; nothing ties them together. This is what
   users pay superwhisper $249 for.
3. **Light mode.** Follow the system appearance. Assets already exist (see 2.4).
4. **Movable HUD position.** Nine-point picker, or at minimum top/bottom. Cheap,
   and the PillFloat story says people care.
5. **A menu-bar popover.** Right now the tray menu is show/quit. Competitors put
   last transcript, mic level, current mode and a pause toggle there. For a
   tray-resident tool, that popover is the real primary surface.
6. **Custom vocabulary import.** Inkwell has a dictionary; superwhisper has the
   same thing with CSV import and calls it a feature. Pure UI affordance.
7. **Onboarding that ends in a win.** Inkwell's forced hotkey test is genuinely
   good and better than most. It should also show the paste landing in a real
   field, which is the moment the product proves itself.

---

## 4. What to remove

- **The permanent 35% ink column.** Keep the identity, spend less window on it:
  a header band, or full-bleed only on the empty Dashboard, or collapsing past a
  width breakpoint.
- **The fake soundwave in the overlay.** Either feed it real amplitude or delete
  the bars. Animated-but-meaningless is worse than static.
- **The 9px and 10px type.** 24 instances, none of them defensible.
- **Advanced Mode as a global switch.** It currently hides two thirds of the app
  behind one toggle. Modes (item 2 above) is the better container: disclosure per
  area, not one master gate.
- **The `Glass*` component names.** They describe backdrop blur the components do
  not have. If light mode and Liquid Glass land, either make them real or rename.

---

## 5. Colour: keep the palette, fix three values

The charcoal-plus-copper direction is good and distinctive. Every competitor is
neutral grey; the warm accent is genuinely Inkwell's. The accent already passes
at 7.32:1. Do not repaint.

Three changes:

```
--color-text-tertiary   0.35 -> 0.55 alpha      (3.17:1 -> ~6:1, fixes AA)
--color-border          0.10 -> 0.14 alpha      (dividers are currently near-invisible)
--color-accent-recording #e84040                 (keep; but it appears almost nowhere,
                                                  the recording state should use it)
```

Then add the light theme by inverting to the cream the ink panel already uses:

```
light  bg-base #f0ede8   surface #ffffff   text #14140f   accent #9c6b3f (darkened for contrast on cream)
dark   unchanged
```

`#c8956c` on cream is only about 2.3:1, so the accent must darken in light mode.
That is the one real trap in the inversion.

---

## 6. Where this leaves the positioning

The field's weak spot is not UI polish, it is trust. The most-cited thread in
current r/macapps dictation discussion is about Wispr Flow routing audio through
third-party servers. superwhisper's complaints are setup complexity and $249.
VoiceInk's UI is described as functional but basic with a steep learning curve.

Inkwell is free, local, and open source, which answers all three. The gap is that
it currently looks less finished than the paid options, and the two reasons for
that are measurable and cheap to fix: the type scale and the tertiary contrast.
Those two changes buy more perceived quality than any new feature on this list.

---

## Sources

Competitive and platform research, July 2026:

- superwhisper Modes documentation and 2026 reviews (per-app auto-activation,
  per-mode shortcuts, custom vocabulary with CSV import, Super Mode screen context)
- Wispr Flow help centre and reviews (menu-bar residency, bottom-centre pill with
  live transcription, onboarding flow, accessibility permissions)
- PillFloat, a third-party utility that exists solely to reposition the Wispr pill
- VoiceInk comparison pages (UI described as functional but basic)
- Handy repository (Tauri + React + Tailwind, tray, configurable overlay position)
- macOS 26 Tahoe / Liquid Glass developer guidance (sidebar tinting, accent
  following, recompile-to-adopt behaviour)
- r/macapps discussion on Wispr Flow's privacy handling
