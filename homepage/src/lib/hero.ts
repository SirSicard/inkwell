/**
 * The hero: one canvas with two orbs, and the coded dictation over them.
 *
 * - The big orb is the main window's, at rest, at 85 % on the app's resting-strength control, wandering in a region the size of the Mac's (Glow.Orb.wander, 0.22 × 0.18 of
 *   the view) centred across the hero and raised a little. It moves in the app's slow live legs (OrbWander: 0.012 of the
 *   view a second, 12 s at least) and holds still between them, drawing nothing while it holds.
 * - The Drop's orb sits in the Drop's circle and goes live while the key is held, its level driven
 *   by the prototype's stand-in voice (the app uses the microphone's level; this page never does).
 * - The loop: the keys go down, the Drop fades in with the live words, the keys come up, the Drop
 *   goes, and the final text is typed into the message. About 11 seconds; then the next sentence.
 *
 * Reduced motion gets one still frame (the shader's `motion` 0) and no loop. The loop pauses while
 * the hero is off screen or the tab is hidden. Without WebGL the CSS glow and the CSS Drop orb stay.
 */
import { MODES, ORB, PRESETS, WANDER_MAC, type Preset } from '../data/glow';
import { DEMO_LINES } from './demo-lines';
import { createOrbCanvas, withShellStrength, hexToRgb, InkMotion, OrbWander, palette, syntheticVoice, type OrbCanvas, type OrbPass } from './glow-orb';

// The resting orb at 85 % on the app's resting-strength control (default 70 %, up to 100 %): above
// 70 % OrbPalette.withShellStrength leans it further toward the dots and raises its coverage, and
// ShellInk's opacity behind text is the strength itself. 100 % left the lead under the glow below
// 3:1; 85 % keeps it readable and still glows.
const STRENGTH = 0.85;
const HOLD = 6; // seconds the big orb holds between legs
const DROP_BG = hexToRgb('#17161e'); // the Drop's background in base.css (.drop)
const W = WANDER_MAC.x[1] - WANDER_MAC.x[0];
// The Mac's region, the same size, centred across the hero and raised by 0.06 so the orb's centre
// stays behind the headline rather than the lead.
const BOUNDS = { x: [0.5 - W / 2, 0.5 + W / 2], y: [WANDER_MAC.y[0] - 0.06, WANDER_MAC.y[1] - 0.06] } as const;

// The loop, in seconds.
const T_DOWN = 1.0, T_UP = 5.4, FADE_IN = 0.25, FADE_OUT = 0.35, T_TYPE = 5.9, PER_CHAR = 0.028, LOOP = 11;

export function startHero(root: HTMLElement) {
  const canvas = root.querySelector<HTMLCanvasElement>('[data-orb]');
  const keys = root.querySelector<HTMLElement>('[data-keys]');
  const drop = root.querySelector<HTMLElement>('[data-drop]');
  const dropOrb = root.querySelector<HTMLElement>('[data-drop-orb]');
  const words = root.querySelector<HTMLElement>('[data-drop-words]');
  const field = root.querySelector<HTMLElement>('[data-field]');
  if (!canvas || !keys || !drop || !dropOrb || !words || !field) return;

  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');
  let preset: Preset = PRESETS[0];
  let pal = palette(preset, MODES.dark, true);
  const strong = () => withShellStrength(pal, STRENGTH);
  let orb: OrbCanvas | null = null;
  const motion = new InkMotion();
  const wander = new OrbWander(BOUNDS, [ORB.drop.x, (BOUNDS.y[0] + BOUNDS.y[1]) / 2]);
  let nextLeg = 1.5; // the first leg starts soon after the page settles
  let clock = 0; // seconds the loop has run
  let orbTime = 0; // the shader's time: advances only on frames that draw, so a hold never jumps
  let running = false, raf = 0, last = 0, visible = true, onScreen = false;
  let dropOpacity = 0;

  const scale = () => Math.min(window.devicePixelRatio || 1, 2) * (window.innerWidth < 640 ? 0.6 : 0.75);

  /** The two passes for this frame, in CSS pixels of the canvas. */
  const passes = (still: boolean): OrbPass[] => {
    const box = canvas.getBoundingClientRect();
    const [fx, fy] = still ? [0.5, (BOUNDS.y[0] + BOUNDS.y[1]) / 2] : wander.position(clock);
    const out: OrbPass[] = [{
      w: [0, 0, 0, 0], you: 0, them: 0,
      x: fx * box.width, y: fy * box.height, unit: ORB.main.unit * Math.min(box.width, box.height), fade: STRENGTH,
    }];
    if (dropOpacity > 0.001) {
      const r = dropOrb.getBoundingClientRect();
      out.push({
        w: [...motion.w], you: motion.envA, them: 0,
        x: r.left - box.left + r.width / 2, y: r.top - box.top + r.height / 2,
        unit: ORB.drop.unit * r.width, fade: dropOpacity, disc: { r: r.width / 2, rgb: DROP_BG },
      });
    }
    return out;
  };

  const draw = (still: boolean) => {
    if (!orb || orb.lost()) return;
    let drawn = false;
    try {
      drawn = orb.draw(passes(still), strong(), orbTime, !still);
    } catch {
      orb = null; // the driver refused the shader: the CSS glow stays
      return;
    }
    if (drawn) root.classList.add('orb-on');
    // Still linking: a still frame (reduced motion, or a resize while paused) tries again shortly.
    else if (!running) window.setTimeout(() => draw(still), 100);
  };

  // The DOM half of the loop, written only when something changed.
  let shown = { keys: true, words: '', field: '\u0000', opacity: -1 };
  const apply = (t: number, line: (typeof DEMO_LINES)[number]) => {
    const down = t >= T_DOWN && t < T_UP;
    if (down !== shown.keys) keys.classList.toggle('is-down', (shown.keys = down));
    dropOpacity = t < T_DOWN ? 0
      : t < T_DOWN + FADE_IN ? (t - T_DOWN) / FADE_IN
      : t < T_UP + 0.3 ? 1
      : Math.max(0, 1 - (t - T_UP - 0.3) / FADE_OUT);
    const o = Math.round(dropOpacity * 100) / 100;
    if (o !== shown.opacity) drop.style.opacity = String((shown.opacity = o));
    const all = line.live.split(' ');
    const from = T_DOWN + 0.35, to = T_UP - 0.25;
    const n = t < from ? 0 : Math.min(all.length, 1 + Math.floor(((t - from) / (to - from)) * all.length));
    const w = all.slice(0, n).join(' ');
    if (w !== shown.words) words.textContent = shown.words = w;
    const f = t < T_TYPE ? '' : line.final.slice(0, Math.floor((t - T_TYPE) / PER_CHAR));
    if (f !== shown.field) field.textContent = shown.field = f;
    motion.state = down ? 'dictating' : 'idle';
    return down;
  };

  const frame = (ts: number) => {
    raf = 0;
    if (!running) return;
    const dt = last ? Math.min(0.1, (ts - last) / 1000) : 1 / 60;
    last = ts;
    clock += dt;
    const t = clock % LOOP;
    const line = DEMO_LINES[Math.floor(clock / LOOP) % DEMO_LINES.length];
    const down = apply(t, line);
    motion.step(dt, down ? syntheticVoice(motion.t, 0.3) : 0, 0);
    if (!wander.isMoving(clock) && clock >= nextLeg) {
      wander.wander(clock);
      nextLeg = Infinity;
    }
    const moving = wander.isMoving(clock);
    if (!moving && nextLeg === Infinity) nextLeg = clock + HOLD;
    if (moving || dropOpacity > 0.001 || !motion.settled) {
      orbTime += dt;
      draw(false);
    }
    raf = requestAnimationFrame(frame);
  };

  const update = () => {
    const go = visible && onScreen && !reduce.matches;
    if (go === running) return;
    running = go;
    if (go) {
      last = 0;
      raf = requestAnimationFrame(frame);
    } else if (raf) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
  };

  /** Reduced motion: the moment mid-dictation, still. */
  const still = () => {
    keys.classList.add('is-down');
    words.textContent = DEMO_LINES[0].live;
    field.textContent = '';
    drop.style.opacity = '1';
    dropOpacity = 1;
    motion.state = 'dictating';
    motion.settle(0.55);
    draw(true);
  };

  const resize = () => {
    if (!orb) return;
    orb.resize(scale());
    if (reduce.matches) still();
    else draw(false);
  };

  // Presets: the orb's colours, and the page's you/them marks.
  root.querySelectorAll<HTMLButtonElement>('[data-preset]').forEach((b) => {
    b.addEventListener('click', () => {
      const next = PRESETS.find((p) => p.id === b.dataset.preset);
      if (!next) return;
      preset = next;
      pal = palette(preset, MODES.dark, true);
      root.querySelectorAll('[data-preset]').forEach((x) => x.setAttribute('aria-pressed', String(x === b)));
      document.documentElement.style.setProperty('--you', preset.you);
      document.documentElement.style.setProperty('--them', preset.them);
      if (reduce.matches) draw(true);
      else draw(false);
    });
  });

  // The loop's first state, before anything runs: nothing pressed, no Drop, an empty field.
  if (!reduce.matches) apply(0, DEMO_LINES[0]);

  try {
    orb = createOrbCanvas(canvas);
  } catch {
    orb = null; // a driver that refuses the shader: the CSS glow stays
  }
  if (orb) {
    orb.resize(scale());
    canvas.addEventListener('webglcontextlost', (e) => {
      e.preventDefault();
      root.classList.remove('orb-on');
      orb = null;
    });
    new ResizeObserver(resize).observe(root);
  }

  if (reduce.matches) still();
  reduce.addEventListener('change', () => {
    if (reduce.matches) still();
    update();
  });
  document.addEventListener('visibilitychange', () => {
    visible = document.visibilityState === 'visible';
    update();
  });
  new IntersectionObserver(([e]) => {
    onScreen = e.isIntersecting;
    update();
  }).observe(root);
}
