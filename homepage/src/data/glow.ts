/**
 * Glow's tokens, as the page uses them: a copy of the values in the app's design/tokens.json (one
 * level up from homepage/), which tests/glow.test.mjs compares against the original, so a change
 * there fails the tests until it is copied here. Vercel builds homepage/ alone, so the page keeps
 * its own copy rather than importing the file.
 *
 * Also the orb's placement and wander region from the Mac shell (mac/Sources/Inkwell/Glow/Glow.swift,
 * Glow.Orb), and the colour fitting both shells share (GlowColours.fit / partner / restTint).
 */

export const MODES = {
  light: {
    background: '#FBF8F4',
    page: '#EDE7DF',
    text: '#1D1B2E',
    secondary: '#6A6577',
    border: '#E2DACE',
    ink: '#1D1B2E',
    idleOrb: '#C7BFDB',
  },
  dark: {
    background: '#121118',
    page: '#0B0A0F',
    text: '#EDEAF2',
    secondary: '#A49FB4',
    border: '#2A2833',
    ink: '#F0EBE3',
    idleOrb: '#5C5470',
  },
} as const;

export const PRESETS = [
  { id: 'indigo', name: 'Indigo & Coral', you: '#6B5CFF', them: '#FFA34D' },
  { id: 'dusk', name: 'Dusk', you: '#406BFF', them: '#F24DC7' },
  { id: 'lagoon', name: 'Lagoon', you: '#1FBFAD', them: '#FFBF47' },
  { id: 'aurora', name: 'Aurora', you: '#4DF2A6', them: '#B366FF' },
  { id: 'citrus', name: 'Citrus', you: '#2E52EB', them: '#FF9E26' },
  { id: 'rosewater', name: 'Rosewater', you: '#998CF2', them: '#FA8CA6' },
  { id: 'ink_sand', name: 'Ink & Sand', you: '#33384D', them: '#D9855C' },
] as const;

export type Preset = (typeof PRESETS)[number];

/** tokens.json "fit": how a dot colour is fitted to a mode, and its lighter partner. */
export const FIT = {
  luma: [0.299, 0.587, 0.114],
  darkLiftBelow: 0.3,
  darkLift: 0.45,
  lightDimAbove: 0.85,
  lightDim: 0.7,
  partnerLift: 0.4,
} as const;

/** tokens.json "orb": where it sits (fractions of the view) and its unit (of the shorter side). */
export const ORB = {
  main: { x: 0.56, y: 0.26, unit: 0.72 },
  drop: { x: 0.5, y: 0.5, unit: 1.15 },
} as const;

/** Glow.Orb.wander: the region of the main window the orb's centre wanders in. */
export const WANDER_MAC = { x: [0.46, 0.68], y: [0.18, 0.36] } as const;

/** GlowColours.restTint: how far the resting orb leans from the idle colour toward the dots. */
export const REST_TINT = 0.6;
