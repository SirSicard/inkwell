/**
 * The page's screenshots. PLACEHOLDERS: every file below is a labelled flat card made by
 * scripts/make-shots.mjs at the size the real capture is served at, so the layout is final. Replace
 * each file in public/shots/ with the real capture at the same name and size (WebP), then delete
 * its `placeholder: true`. The ids are those of the 1.0 homepage audit's shot list (2026-10-09).
 *
 * Capture rules (from the audit): a seeded demo library in a scratch INK_DATA_DIR with invented
 * names, never real meetings; an English locale for numbers and dates; Mac at 2x from a
 * 1440 × 900 pt window; Windows 11 24H2 at 150 %.
 *
 *   file (public/shots/)                    served size   source capture                     section
 *   d1-drop-states-{light,dark}.webp        880 × 640     the Drop's four states, Mac, 2x    Dictation
 *   m1-live-dark.webp                       1440 × 900    Live screen mid-meeting, Mac       Meetings (dark band)
 *   l1-library-{light,dark}.webp            1440 × 900    Library with a search match, Mac   Library
 *   o1-owed-{light,dark}.webp               1440 × 900    Owed: overdue, "looks done", Undo  Library (wide screens)
 *   s2-share-card-{light,dark}.webp         1040 × 1282   the Stats share card, Mac          Stats
 *   p2-settings-ai-{light,dark}.webp        1440 × 900    Settings > AI, Local only on       Models
 *   p3-consent-{light,dark}.webp            1040 × 640    the Polish consent prompt, sheet   Privacy
 *   w1-smartscreen.webp                     1100 × 700    SmartScreen + Get-FileHash, Win    Install > Windows
 *
 * Not on the page: V1/V2 (the hero is a coded animation over the WebGL orb), D2 (the Drop's offer is
 * drawn in HTML in Consent), G1 (the orb is the live shader), I1 and OG (scripts/make-icons.mjs,
 * scripts/make-og.mjs), and M2, M3, T1, S1, P1, F1, F2 (left out to keep the phone page short).
 */
export type Shot = {
  id: string;
  /** File stem in public/shots/; `-light` / `-dark` is added per theme. */
  file: string;
  width: number;
  height: number;
  /** Which themes exist: both, or one used whatever the page's theme. */
  themes: 'both' | 'dark' | 'light';
  alt: string;
  placeholder: boolean;
};

export const SHOTS = {
  d1: { id: 'D1', file: 'd1-drop-states', width: 880, height: 640, themes: 'both', placeholder: true,
    alt: 'The Drop in its four states: dictating with live words, recording a meeting, the final pass, and a problem.' },
  m1: { id: 'M1', file: 'm1-live', width: 1440, height: 900, themes: 'dark', placeholder: true,
    alt: 'The Live screen during a meeting: the transcript in two colours, notes beside it, and Ask with one question answered.' },
  l1: { id: 'L1', file: 'l1-library', width: 1440, height: 900, themes: 'both', placeholder: true,
    alt: 'The Library: meetings and dictations together, with a search term matched inside a transcript.' },
  o1: { id: 'O1', file: 'o1-owed', width: 1440, height: 900, themes: 'both', placeholder: true,
    alt: 'Owed: open promises grouped by meeting, one overdue, one marked as looking done, and an Undo toast.' },
  s2: { id: 'S2', file: 's2-share-card', width: 1040, height: 1282, themes: 'both', placeholder: true,
    alt: 'The Stats share card: words dictated, time saved and meeting hours, counted on this Mac.' },
  p2: { id: 'P2', file: 'p2-settings-ai', width: 1440, height: 900, themes: 'both', placeholder: true,
    alt: 'Settings > AI with Local only on and Apple’s on-device model available.' },
  p3: { id: 'P3', file: 'p3-consent', width: 1040, height: 640, themes: 'both', placeholder: true,
    alt: 'The prompt Polish shows before its first send, asking whether text goes to this Mac or to Groq.' },
  w1: { id: 'W1', file: 'w1-smartscreen', width: 1100, height: 700, themes: 'light', placeholder: true,
    alt: 'Windows SmartScreen’s “Windows protected your PC” with More info open, beside PowerShell’s Get-FileHash output.' },
} as const satisfies Record<string, Shot>;
