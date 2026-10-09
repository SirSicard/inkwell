/**
 * The page's screenshots: real Mac captures of a seeded demo library (invented names, never real
 * meetings), except W1, which is still a PLACEHOLDER until it is captured on the PC.
 *
 * A real shot is two WebP files per theme in public/shots/, `<stem>-1x.webp` and `<stem>-2x.webp`,
 * served through srcset. `width` × `height` is the 2x file; the 1x is exactly half. They are made by
 * scripts/encode-shots.mjs from the raw captures (2880 × 1800 PNGs of a 1440 × 900 pt window, with
 * alpha corners), which stay out of the repository. A placeholder is one flat card, `<stem>.webp`,
 * made by scripts/make-shots.mjs at the size the capture will be served at.
 *
 * Capture rules (from the 1.0 homepage audit, 2026-10-09): a seeded demo library in a scratch
 * INK_DATA_DIR with invented names; an English locale for numbers and dates; Mac at 2x from a
 * 1440 × 900 pt window; Windows 11 24H2 at 150 %.
 *
 *   shot   files (public/shots/)              2x size       shows                               section
 *   D1     d1-drop-{meeting,final}-dark        890 × 162     the Drop recording; final pass      Meetings (night band)
 *   M1     m1-live-dark                        1792 × 1120   Live screen mid-meeting             Meetings (night band)
 *   L1     l1-library-{light,dark}             1152 × 720    a meeting record in the Library     Library
 *   O1     o1-owed-{light,dark}                1152 × 720    Owed: overdue, "Looks done", Undo   Library
 *   S1     s1-stats-{light,dark}               1152 × 720    the Stats screen                    Library > Stats
 *   S2     s2-share-card-{light,dark}          704 × 776     the Stats share card                Library > Stats
 *   P2     p2-settings-ai-{light,dark}         1152 × 720    Settings > AI, Local only on        Models
 *   P3     p3-consent-dark                     1040 × 640    the "Turn on polish?" alert         Privacy (night band)
 *   W1     w1-smartscreen.webp (placeholder)   1100 × 700    SmartScreen + Get-FileHash, Win     Install > Windows
 *
 * Every shot is hidden below tablet width (`optional`), so none of them adds to the phone page.
 * Not on the page: T1 (Today exists only dark, and the night band already carries M1 and the Drops),
 * V1/V2 (the hero is a coded animation over the WebGL orb), D2 (the Drop's offer is drawn in HTML in
 * Consent), G1 (the orb is the live shader), I1 and OG (scripts/make-icons.mjs, scripts/make-og.mjs),
 * and M2, M3, P1, F1, F2.
 */
export type Shot = {
  id: string;
  /** File stem in public/shots/; `-light` / `-dark` is added per theme. */
  file: string;
  /** The 2x file's size in pixels (a placeholder's only file); the 1x is half. */
  width: number;
  height: number;
  /** Which themes exist: both, or one used whatever the page's theme. */
  themes: 'both' | 'dark' | 'light';
  alt: string;
  placeholder: boolean;
};

export const SHOTS = {
  d1m: { id: 'D1-meeting', file: 'd1-drop-meeting', width: 890, height: 162, themes: 'dark', placeholder: false,
    alt: 'The Drop while a meeting records: REC lit, and the latest words appearing as they are said.' },
  d1f: { id: 'D1-final', file: 'd1-drop-final', width: 890, height: 162, themes: 'dark', placeholder: false,
    alt: 'The Drop after the meeting, blotting: the final pass is running.' },
  m1: { id: 'M1', file: 'm1-live', width: 1792, height: 1120, themes: 'dark', placeholder: false,
    alt: 'The Live screen 37 seconds into a meeting: your notes on the left, the transcript in two colours on the right with its newest line still settling in grey, and a question asked of you above the Ask field.' },
  l1: { id: 'L1', file: 'l1-library', width: 1152, height: 720, themes: 'both', placeholder: false,
    alt: 'The Library with the “Kestrel 2.0 launch readiness” meeting open: dictations and meetings newest first, your notes filled in beside the transcript in two colours, and one player for both sides.' },
  o1: { id: 'O1', file: 'o1-owed', width: 1152, height: 720, themes: 'both', placeholder: false,
    alt: 'Owed: open promises grouped by who they are owed to, two of them overdue, a “Looks done” suggestion with Mark done and Not yet, and an Undo toast for one just marked done.' },
  s1: { id: 'S1', file: 's1-stats', width: 1152, height: 720, themes: 'both', placeholder: false,
    alt: 'Stats: last week’s review (420 words, 8 minutes saved, 1 h 44 min in 3 meetings, 4 promises kept) and the dictation counts with a 12-week heatmap of active days.' },
  s2: { id: 'S2', file: 's2-share-card', width: 704, height: 776, themes: 'both', placeholder: false,
    alt: 'The Stats share card: 4,060 words dictated, 1 h 14 min saved against typing at 40 wpm, a 22-day streak, 1 h 53 min in meetings this month and two milestones, counted on my Mac.' },
  p2: { id: 'P2', file: 'p2-settings-ai', width: 1152, height: 720, themes: 'both', placeholder: false,
    alt: 'Settings > AI: no language model chosen, Local only on, and Polish, voice edit, and summaries and Ask each off until you turn them on.' },
  p3: { id: 'P3', file: 'p3-consent', width: 1040, height: 640, themes: 'dark', placeholder: false,
    alt: 'The “Turn on polish?” alert over Settings: Polish sends what you dictate to a language model, here Apple’s on-device one, so your words stay on this Mac. Turn On Polish or Cancel.' },
  w1: { id: 'W1', file: 'w1-smartscreen', width: 1100, height: 700, themes: 'light', placeholder: true,
    alt: 'Windows SmartScreen’s “Windows protected your PC” with More info open, beside PowerShell’s Get-FileHash output.' },
} as const satisfies Record<string, Shot>;
