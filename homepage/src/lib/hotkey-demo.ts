/*
  The hotkey loop, re-enacted: hold the key, the ink swells with a voice, let go, the sentence lands.

  PLACEHOLDER: the voice is a synthetic amplitude envelope built from the
  sentence's syllables, and the sentences are transcripts shown in the app's own dashboard
  screenshot. The finished page swaps in one real recording's envelope and its real transcript.
  The page never touches the microphone, and says so under the demo.

  Timing is setInterval/setTimeout, not requestAnimationFrame, so the sentence still lands on a
  page whose rAF never fires (the hostile-environment gate). The ink's own loop is rAF and pauses.
*/

import type { InkField } from './ink-field';

type Envelope = { dur: number; at: (t: number) => number };

function syllables(word: string) {
  const w = word.toLowerCase().replace(/[^a-z]/g, '').replace(/e$/, '');
  const groups = w.match(/[aeiouy]+/g);
  return Math.max(1, groups ? groups.length : 1);
}

/** A plausible speech envelope for a sentence: one bump per syllable, pauses at punctuation. */
export function envelope(text: string, seed: number): Envelope {
  let s = (seed * 7919 + 104729) % 233280;
  const rnd = () => (s = (s * 9301 + 49297) % 233280) / 233280;
  const events: { t0: number; d: number; a: number }[] = [];
  let t = 0.14; // key down, a breath, then speech
  const words = text.split(/\s+/).filter(Boolean);
  words.forEach((word, i) => {
    for (let k = 0, n = syllables(word); k < n; k++) {
      const d = 0.15 + rnd() * 0.09;
      events.push({ t0: t, d, a: 0.5 + rnd() * 0.5 });
      t += d * 0.92;
    }
    t += 0.035;
    if (/[,;:]$/.test(word)) t += 0.2;
    if (/[.?!]$/.test(word) && i < words.length - 1) t += 0.34;
  });
  const dur = t + 0.05;
  return {
    dur,
    at(x) {
      if (x <= 0 || x >= dur) return 0;
      let v = 0;
      for (const e of events) {
        const u = (x - e.t0) / e.d;
        if (u > 0 && u < 1) v = Math.max(v, e.a * Math.sin(Math.PI * u) ** 1.5);
      }
      return Math.min(1, v * (1 - 0.22 * (x / dur))); // phrase declination
    },
  };
}

type Parts = {
  root: HTMLElement;
  key: HTMLButtonElement;
  field: HTMLElement;
  caret: HTMLElement;
  status: HTMLElement;
  ink: InkField | null;
};

const KEYS = new Set([' ', 'Enter', 'Spacebar']);
const MAX_LINES = 3; // sentences kept in the field; older ones scroll out of its fixed box
const MIN_HOLD = 0.35; // seconds; a shorter press counts as a tap and replays the whole sentence

export function hotkeyDemo(p: Parts, sentences: string[]) {
  const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');
  let next = 1; // sentences[0] is already in the server HTML
  let phase: 'idle' | 'down' | 'landing' = 'idle';
  let auto = false;
  let t0 = 0;
  let tick = 0;
  let env: Envelope = envelope(sentences[next % sentences.length], next);
  let ignoreClickUntil = 0;

  const say = (msg: string) => {
    p.status.textContent = msg;
  };
  const level = (v: number) => p.ink?.setLevel?.(v);

  const press = (isAuto: boolean) => {
    if (phase !== 'idle') return;
    phase = 'down';
    auto = isAuto;
    env = envelope(sentences[next % sentences.length], next);
    p.root.classList.add('is-recording');
    say('Key down. Inkwell records while you hold it.');
    t0 = performance.now();
    if (reduce.matches) level(0.8); // one swollen still, no tween
    tick = window.setInterval(() => {
      const t = (performance.now() - t0) / 1000;
      if (!reduce.matches) level(env.at(t));
      if (auto && t >= env.dur + 0.12) release(true);
    }, 30);
  };

  const release = (fromTimer = false) => {
    if (phase !== 'down') return;
    if (!fromTimer) {
      if (auto) return; // a replay finishes on its own timer
      if ((performance.now() - t0) / 1000 < MIN_HOLD) {
        auto = true; // a tap: let the sentence finish on its own
        return;
      }
    }
    window.clearInterval(tick);
    phase = 'landing';
    p.root.classList.remove('is-recording');
    level(0);
    say('Key up. Transcribing on this machine.');
    window.setTimeout(land, reduce.matches ? 0 : 320);
  };

  const land = () => {
    const text = sentences[next % sentences.length];
    next += 1;
    const span = document.createElement('span');
    span.className = 'landed';
    span.textContent = ` ${text}`;
    p.field.insertBefore(span, p.caret);
    // The field has a fixed box (no layout shift): drop the oldest sentences until the newest fits.
    const box = p.field.parentElement;
    let spans = p.field.querySelectorAll('.landed, .seed');
    while (spans.length > 1 && (spans.length > MAX_LINES || (box && box.scrollHeight > box.clientHeight + 1))) {
      spans[0].remove();
      spans = p.field.querySelectorAll('.landed, .seed');
    }
    // Restart the caret's short blink (it stops after a few beats).
    p.caret.classList.remove('blink');
    void p.caret.offsetWidth;
    p.caret.classList.add('blink');
    say('Pasted where the cursor was.');
    phase = 'idle';
  };

  // Pointer: hold for as long as you like; a quick tap replays the whole sentence.
  p.key.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    try {
      p.key.setPointerCapture(e.pointerId);
    } catch {
      /* capture is a nicety */
    }
    p.key.dataset.pointer = ''; // hides the focus ring until the keyboard is used (base.css)
    p.key.focus({ preventScroll: true });
    press(false);
  });
  const up = () => {
    ignoreClickUntil = performance.now() + 400;
    release();
  };
  p.key.addEventListener('pointerup', up);
  p.key.addEventListener('pointercancel', up);
  p.key.addEventListener('lostpointercapture', up);
  p.key.addEventListener('contextmenu', (e) => e.preventDefault());

  // Keyboard: focus the key and hold Space or Enter, like the real hotkey.
  p.key.addEventListener('keydown', (e) => {
    delete p.key.dataset.pointer; // the keyboard is in use: show the ring again
    if (!KEYS.has(e.key)) return;
    e.preventDefault();
    if (!e.repeat) press(false);
  });
  p.key.addEventListener('keyup', (e) => {
    if (!KEYS.has(e.key)) return;
    e.preventDefault();
    ignoreClickUntil = performance.now() + 400;
    release();
  });
  p.key.addEventListener('blur', () => {
    delete p.key.dataset.pointer;
    release();
  });

  // Assistive tech often sends a bare click with no key or pointer events: replay the sentence.
  p.key.addEventListener('click', () => {
    if (performance.now() < ignoreClickUntil || phase !== 'idle') return;
    press(true);
  });

  p.key.disabled = false;

  return {
    /** Runs the loop once on its own: the page's one orchestrated moment. */
    autoplay() {
      if (phase === 'idle' && !reduce.matches && !document.hidden) press(true);
    },
  };
}
