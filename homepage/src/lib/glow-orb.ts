/**
 * The Glow orb in WebGL: a port of the app's shader (shaders/ink.wgsl, in the app's repository root)
 * to GLSL ES 1.0, with the parts of the Mac renderer that drive it: the colour fitting
 * (mac/Sources/Inkwell/Glow/Glow.swift, GlowColours), the state weights' easing and the level
 * envelope (mac/Sources/InkRenderer/InkSimulation.swift), the prototype's stand-in voice, and the
 * wander path (mac/Sources/InkRenderer/OrbWander.swift).
 *
 * Differences from ink.wgsl, all additions for the page:
 *   1. gl_FragCoord has a bottom-left origin; it is flipped to the WGSL top-left `pos` first, so
 *      `center` and the grain's hash read the same pixels as in the app.
 *   2. `fade` scales the premultiplied output: the orb's opacity behind text (the app sets it on
 *      the view as alphaValue; one canvas here draws two orbs, so it is a uniform).
 *   3. `disc` (rgb, radius in pixels; radius 0 is off): an opaque disc under the orb, and the orb
 *      clipped to it. That is the Drop's orb in its circle (the app's orbHolder clips with
 *      masksToBounds) drawn on the same canvas as the big orb behind it.
 * The uniform block is otherwise ink.wgsl's G, field for field.
 */
import { FIT, REST_TINT, type Preset } from '../data/glow';

// The shader source keeps webgl-noise's licence notice: it ships with the snoise() it covers.
const FRAG = `/* snoise() is 2D simplex noise from webgl-noise, by Ian McEwan, Ashima Arts
(https://github.com/ashima/webgl-noise, maintained at https://github.com/stegu/webgl-noise), MIT licensed.
Copyright (C) 2011 by Ashima Arts (Simplex noise)
Copyright (C) 2011-2016 by Stefan Gustavson (Classic noise and others)
Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense,
and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:
The above copyright notice and this permission notice shall be included in all copies or substantial
portions of the Software.
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT
LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE. */
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
uniform vec2 res;
uniform vec2 center;
uniform float time;
uniform float unit;
uniform float you;
uniform float them;
uniform vec4 w;
uniform float dark;
uniform float motion;
uniform vec3 yA;
uniform vec3 yB;
uniform vec3 tA;
uniform vec3 tB;
uniform vec4 idle;
uniform vec4 ink;
uniform float fade;
uniform vec4 disc;

vec2 mod289(vec2 x) { return x - 289.0 * floor(x / 289.0); }
vec3 mod289(vec3 x) { return x - 289.0 * floor(x / 289.0); }
vec3 permute(vec3 x) { return mod289(((x * 34.0) + 1.0) * x); }

float snoise(vec2 v) {
  const vec4 C = vec4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
  vec2 i = floor(v + dot(v, C.yy));
  vec2 x0 = v - i + dot(i, C.xx);
  vec2 i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
  vec4 x12 = x0.xyxy + C.xxzz;
  x12.xy -= i1;
  i = mod289(i);
  vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
  vec3 m = max(0.5 - vec3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), 0.0);
  m = m * m;
  m = m * m;
  vec3 x = 2.0 * fract(p * C.www) - 1.0;
  vec3 h = abs(x) - 0.5;
  vec3 ox = floor(x + 0.5);
  vec3 a0 = x - ox;
  m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
  vec3 g = vec3(a0.x * x0.x + h.x * x0.y, a0.yz * x12.xz + h.yz * x12.yw);
  return 130.0 * dot(m, g);
}

float fbm(vec2 q) {
  vec2 p = q;
  float v = 0.0;
  float a = 0.5;
  for (int i = 0; i < 5; i++) {
    v += a * snoise(p);
    p = p * 2.03 + vec2(1.7, 9.2);
    a *= 0.5;
  }
  return 0.48 + 0.455 * v;
}

float hash(vec2 p) { return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453); }

vec4 orb(vec2 q, float R, float soft, vec3 A, vec3 B, float seed, float tt) {
  float l = length(q);
  if (l > R + soft + 0.03) return vec4(0.0);
  float a = 1.0 - smoothstep(R - soft * 0.5, R + soft, l + 0.05 * (fbm(q * 3.0 + seed + tt * 0.2) - 0.5));
  float h = fbm(q * 2.2 + vec2(tt * 0.15, seed));
  vec3 c = mix(A, B, smoothstep(0.3, 0.7, h));
  float d = length(q - vec2(-0.25, 0.3) * R);
  float shine = R > 0.0 ? 1.0 - smoothstep(0.0, R, d) : 1.0;
  c += 0.22 * (1.0 - 0.4 * dark) * shine;
  return vec4(c, a);
}

vec3 greyed(vec3 c, float k) { return mix(c, vec3(dot(c, vec3(0.299, 0.587, 0.114))), k); }

void main() {
  vec2 pos = vec2(gl_FragCoord.x, res.y - gl_FragCoord.y);
  vec2 p = vec2(pos.x - center.x, center.y - pos.y) / unit;
  float inDisc = disc.w > 0.0 ? 1.0 - smoothstep(disc.w - 1.0, disc.w, length(pos - center)) : 1.0;
  float dictating = w.x;
  float meeting = w.y;
  float blot = w.z;
  float problem = w.w;
  float live = max(dictating, meeting);
  float t = time * motion;
  float tt = t * (0.12 + 0.88 * live);
  float apart = meeting * (1.0 - blot);
  vec2 swirl = 0.09 * vec2(cos(tt * 0.5), sin(tt * 0.5));
  vec2 c1 = (vec2(-0.15, 0.02) + swirl) * apart;
  vec2 c2 = (vec2(0.15, -0.02) - swirl) * apart;
  float pulse = 0.5 + 0.5 * sin(t * 2.0);
  float breath = 1.0 + 0.06 * blot * sin(t * 1.6);
  float r1 = mix((0.19 + 0.05 * dictating + 0.09 * you * live) * mix(0.8, 1.0, live), 0.13 * breath, blot);
  float r2 = mix((0.16 + 0.08 * them) * meeting, 0.0, blot) * (1.0 + 0.06 * problem * (pulse - 0.5));
  float soft = mix(0.24, 0.025, blot);
  float restTint = clamp(idle.a, 0.0, 1.0);
  vec3 restA = mix(idle.rgb, yA, restTint);
  vec3 restB = mix(idle.rgb, tA, restTint);
  vec4 o1 = orb(p - c1, r1, soft,
    mix(mix(restA, yA, live), ink.rgb, blot),
    mix(mix(restB, yB, live), ink.rgb, blot), 0.0, tt);
  vec4 o2 = vec4(0.0);
  if (meeting > 0.0) {
    o2 = orb(p - c2, r2, soft,
      mix(greyed(tA, 0.85 * problem), ink.rgb, blot),
      mix(greyed(tB, 0.85 * problem), ink.rgb, blot), 3.0, tt);
  }
  float a2 = o2.a * 0.9 * meeting * (1.0 - problem * (0.45 - 0.3 * pulse));
  float restAlpha = mix(0.55, 0.825, clamp(ink.a, 0.0, 1.0));
  float a1 = o1.a * 0.9 * mix(restAlpha, 1.0, max(live, blot)) * (1.0 - 0.35 * a2 * (1.0 - blot));
  float alpha = (a1 + a2 * (1.0 - a1)) * inDisc;
  vec3 col = vec3(0.0);
  if (alpha > 0.0) {
    col = (o1.rgb * a1 + o2.rgb * a2 * (1.0 - a1)) / max(a1 + a2 * (1.0 - a1), 0.0001);
    col += (hash(pos + fract(t)) - 0.5) * 0.05;
    col = clamp(col, 0.0, 1.0);
  }
  vec4 outc = vec4(col * alpha, alpha);
  if (disc.w > 0.0) outc = outc + vec4(disc.rgb, 1.0) * inDisc * (1.0 - alpha);
  gl_FragColor = outc * fade;
}
`;

const VERT = `attribute vec2 a;
void main() { gl_Position = vec4(a, 0.0, 1.0); }
`;

export type RGB = [number, number, number];

export function hexToRgb(hex: string): RGB {
  const n = parseInt(hex.slice(1), 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

const luminance = (c: RGB) => FIT.luma[0] * c[0] + FIT.luma[1] * c[1] + FIT.luma[2] * c[2];
const lift = (c: RGB, k: number): RGB => [c[0] + (1 - c[0]) * k, c[1] + (1 - c[1]) * k, c[2] + (1 - c[2]) * k];

/** GlowColours.fit: too dark for night is lifted; too pale for day is deepened. */
export function fit(c: RGB, dark: boolean): RGB {
  if (dark && luminance(c) < FIT.darkLiftBelow) return lift(c, FIT.darkLift);
  if (!dark && luminance(c) > FIT.lightDimAbove) return [c[0] * FIT.lightDim, c[1] * FIT.lightDim, c[2] * FIT.lightDim];
  return c;
}

/** GlowColours.partner: the orb's second shade, the same colour lighter. */
export const partner = (c: RGB): RGB => lift(c, FIT.partnerLift);

export type Palette = { yA: RGB; yB: RGB; tA: RGB; tB: RGB; idle: RGB; ink: RGB; dark: boolean; restTint: number; restBoost: number };

/** GlowColours.palette for one preset and mode: `idle` and `ink` are the mode's idleOrb and ink. */
export function palette(preset: Preset, mode: { idleOrb: string; ink: string }, dark: boolean): Palette {
  const you = fit(hexToRgb(preset.you), dark);
  const them = fit(hexToRgb(preset.them), dark);
  return {
    yA: you, yB: partner(you), tA: them, tB: partner(them),
    idle: hexToRgb(mode.idleOrb), ink: hexToRgb(mode.ink), dark, restTint: REST_TINT, restBoost: 0,
  };
}

/** OrbPalette.withShellStrength: above 70 %, the resting orb eases into a stronger tint and coverage. */
export function withShellStrength(p: Palette, strength: number): Palette {
  const k = Math.min(1, Math.max(0, (strength - 0.7) / 0.3));
  const boost = k * k * (3 - 2 * k);
  return { ...p, restTint: p.restTint + (1 - p.restTint) * boost, restBoost: boost };
}

/** One orb to draw: its state and where it sits, in CSS pixels of the canvas. */
export type OrbPass = {
  /** Dictating, meeting, blotting, problem; each 0..1. */
  w: [number, number, number, number];
  you: number;
  them: number;
  /** Centre, CSS px from the canvas's top left, and the unit in CSS px. */
  x: number;
  y: number;
  unit: number;
  /** Opacity of the whole pass. */
  fade: number;
  /** The Drop's circle: its radius in CSS px and colour, or none. */
  disc?: { r: number; rgb: RGB };
};

const UNIFORMS = ['res', 'center', 'time', 'unit', 'you', 'them', 'w', 'dark', 'motion', 'yA', 'yB', 'tA', 'tB', 'idle', 'ink', 'fade', 'disc'] as const;

/** A canvas that draws orbs, or null when WebGL is unavailable (the page's CSS glow stays). */
export function createOrbCanvas(canvas: HTMLCanvasElement) {
  const gl = canvas.getContext('webgl', { alpha: true, premultipliedAlpha: true, antialias: false, depth: false, stencil: false, powerPreference: 'low-power' });
  if (!gl) return null;
  // Compile and link without waiting: with KHR_parallel_shader_compile the driver links off the main
  // thread and `ready()` polls for it, so a slow compiler (SwiftShader takes over a second) never
  // blocks the page. Without the extension the first `ready()` waits, as WebGL always did.
  const parallel = gl.getExtension('KHR_parallel_shader_compile');
  const shader = (type: number, src: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    return s;
  };
  const vs = shader(gl.VERTEX_SHADER, VERT);
  const fs = shader(gl.FRAGMENT_SHADER, FRAG);
  const prog = gl.createProgram()!;
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  let u: Record<(typeof UNIFORMS)[number], WebGLUniformLocation | null> | null = null;

  /** True once the program is linked and set up; throws if the driver refused the shader. */
  const ready = () => {
    if (u) return true;
    if (parallel && !gl.getProgramParameter(prog, parallel.COMPLETION_STATUS_KHR)) return false;
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      throw new Error(gl.getShaderInfoLog(fs) || gl.getProgramInfoLog(prog) || 'the orb shader did not link');
    }
    gl.useProgram(prog);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    // The vertex stage's full-screen triangle strip: (-1,-1), (1,-1), (-1,1), (1,1).
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
    const loc = gl.getAttribLocation(prog, 'a');
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
    gl.enable(gl.BLEND);
    // Premultiplied output, blended one and one minus source alpha over what is behind.
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.enable(gl.SCISSOR_TEST);
    u = Object.fromEntries(UNIFORMS.map((n) => [n, gl.getUniformLocation(prog, n)])) as NonNullable<typeof u>;
    return true;
  };

  let scale = 1;

  return {
    /** Sizes the drawing buffer to the canvas's CSS box times `pixelScale`. */
    resize(pixelScale: number) {
      const r = canvas.getBoundingClientRect();
      scale = pixelScale;
      const w = Math.max(2, Math.round(r.width * scale));
      const h = Math.max(2, Math.round(r.height * scale));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
      gl.viewport(0, 0, w, h);
    },
    ready,
    /** Clears, then draws each pass in order (later ones over earlier ones). False until ready. */
    draw(passes: OrbPass[], pal: Palette, time: number, motion: boolean): boolean {
      if (!ready() || !u) return false;
      const W = canvas.width, H = canvas.height;
      gl.scissor(0, 0, W, H);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.uniform2f(u.res, W, H);
      gl.uniform1f(u.time, time);
      gl.uniform1f(u.dark, pal.dark ? 1 : 0);
      gl.uniform1f(u.motion, motion ? 1 : 0);
      gl.uniform3fv(u.yA, pal.yA);
      gl.uniform3fv(u.yB, pal.yB);
      gl.uniform3fv(u.tA, pal.tA);
      gl.uniform3fv(u.tB, pal.tB);
      gl.uniform4f(u.idle, ...pal.idle, Math.min(1, Math.max(0, pal.restTint)));
      gl.uniform4f(u.ink, ...pal.ink, Math.min(1, Math.max(0, pal.restBoost)));
      for (const p of passes) {
        if (p.fade <= 0.001) continue;
        const cx = p.x * scale, cy = p.y * scale, unit = p.unit * scale;
        // Only the pixels the orb (at most R + soft + 0.03 units, with R up to 0.33) or the disc can reach.
        const reach = p.disc ? p.disc.r * scale + 2 : 0.66 * unit;
        const x0 = Math.max(0, Math.floor(cx - reach)), y0 = Math.max(0, Math.floor(H - cy - reach));
        const x1 = Math.min(W, Math.ceil(cx + reach)), y1 = Math.min(H, Math.ceil(H - cy + reach));
        if (x1 <= x0 || y1 <= y0) continue;
        gl.scissor(x0, y0, x1 - x0, y1 - y0);
        gl.uniform2f(u.center, cx, cy);
        gl.uniform1f(u.unit, unit);
        gl.uniform1f(u.you, p.you);
        gl.uniform1f(u.them, p.them);
        gl.uniform4f(u.w, ...p.w);
        gl.uniform1f(u.fade, p.fade);
        gl.uniform4f(u.disc, ...(p.disc ? p.disc.rgb : ([0, 0, 0] as RGB)), p.disc ? p.disc.r * scale : 0);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      }
      return true;
    },
    lost: () => gl.isContextLost(),
  };
}

export type OrbCanvas = NonNullable<ReturnType<typeof createOrbCanvas>>;

/* ── The ink's motion (InkSimulation.swift), the parts the orb reads ──────────────────────────── */

/** The prototype's `_sm`: a smoothstep over -0.25...0.25. */
function sm(x: number) {
  const u = Math.max(0, Math.min(1, (x + 0.25) / 0.5));
  return u * u * (3 - 2 * u);
}

/** The prototype's stand-in voice: syllables near 4 Hz, grouped into phrases with pauses. */
export function syntheticVoice(t: number, seed: number) {
  const syl = Math.pow(Math.max(0, Math.sin(t * 2 * Math.PI * 4.1 + seed * 5)), 2.2);
  const phrase = sm(Math.sin(t * 0.9 + seed * 2) + 0.6 * Math.sin(t * 2.3 + seed) + 0.35);
  return syl * phrase * (0.62 + 0.38 * Math.sin(t * 13.7 + seed * 9));
}

export type InkState = 'idle' | 'dictating' | 'meeting';

const WEIGHTS: Record<InkState, [number, number, number, number]> = {
  idle: [0, 0, 0, 0],
  dictating: [1, 0, 0, 0],
  meeting: [0, 1, 0, 0],
};

/** The weights eased toward the state's (0.04 per 60 Hz frame), and the levels' envelope follower. */
export class InkMotion {
  t = 0;
  w: [number, number, number, number] = [0, 0, 0, 0];
  envA = 0;
  envB = 0;
  state: InkState = 'idle';

  step(dt: number, near: number, far: number) {
    this.t += dt;
    const vA = this.state === 'idle' ? 0 : near;
    const vB = this.state === 'meeting' ? far : 0;
    // Envelope follower: fast attack, slow release, the way a level meter reads a voice.
    const atk = 1 - Math.exp(-dt * 28), rel = 1 - Math.exp(-dt * 6);
    this.envA += (vA - this.envA) * (vA > this.envA ? atk : rel);
    this.envB += (vB - this.envB) * (vB > this.envB ? atk : rel);
    const kw = 1 - Math.pow(1 - 0.04, dt * 60);
    const target = WEIGHTS[this.state];
    for (let i = 0; i < 4; i++) this.w[i] += (target[i] - this.w[i]) * kw;
  }

  /** The still frame: every spring at its target. */
  settle(near: number) {
    this.w = [...WEIGHTS[this.state]];
    this.envA = this.state === 'idle' ? 0 : near;
    this.envB = 0;
  }

  /** Whether anything still moves at rest (the weights still easing). */
  get settled() {
    const target = WEIGHTS[this.state];
    return this.state === 'idle' && this.envA < 0.002 && this.w.every((v, i) => Math.abs(v - target[i]) < 0.002);
  }
}

/* ── OrbWander.swift ─────────────────────────────────────────────────────────────────────────── */

type V2 = [number, number];
export type Bounds = { x: readonly [number, number]; y: readonly [number, number] };

/** Smootherstep legs between random spots in `bounds` (fractions of the view). */
export class OrbWander {
  /** OrbWander.cruise and shortestLeg: a live leg's mean speed (fractions per second) and floor. */
  static cruise = 0.012;
  static shortestLeg = 12;
  static minimumHop = 1 / 3;
  readonly bounds: Bounds;
  private random: () => number;
  private from: V2;
  private to: V2;
  private start = 0;
  private duration = 0;

  // No parameter properties: the file stays erasable TypeScript, so make-og.mjs can strip it.
  constructor(bounds: Bounds, start: V2, random: () => number = Math.random) {
    this.bounds = bounds;
    this.random = random;
    const p = this.clamped(start);
    this.from = p;
    this.to = p;
  }

  private clamped(p: V2): V2 {
    const { x, y } = this.bounds;
    return [Math.min(x[1], Math.max(x[0], p[0])), Math.min(y[1], Math.max(y[0], p[1]))];
  }

  position(t: number): V2 {
    if (this.duration <= 0) return this.to;
    const u = Math.min(1, Math.max(0, (t - this.start) / this.duration));
    const e = u * u * u * (u * (u * 6 - 15) + 10); // smootherstep: legs join without a kink
    return [this.from[0] + (this.to[0] - this.from[0]) * e, this.from[1] + (this.to[1] - this.from[1]) * e];
  }

  isMoving(t: number) {
    return this.duration > 0 && t < this.start + this.duration;
  }

  /** A slow leg toward a new spot, as the app's live wander takes. */
  wander(t: number) {
    if (this.isMoving(t)) return;
    const from = this.position(t);
    const to = this.nextSpot(from);
    const distance = Math.hypot(to[0] - from[0], to[1] - from[1]);
    this.from = from;
    this.to = to;
    this.start = t;
    this.duration = Math.max(OrbWander.shortestLeg, distance / OrbWander.cruise);
  }

  /** The first of eight random spots at least minimumHop of the diagonal away, else the farthest. */
  private nextSpot(from: V2): V2 {
    const { x, y } = this.bounds;
    const hop = OrbWander.minimumHop * Math.hypot(x[1] - x[0], y[1] - y[0]);
    let best = from, bestDistance = -1;
    for (let i = 0; i < 8; i++) {
      const p: V2 = [x[0] + this.random() * (x[1] - x[0]), y[0] + this.random() * (y[1] - y[0])];
      const d = Math.hypot(p[0] - from[0], p[1] - from[1]);
      if (d >= hop) return p;
      if (d > bestDistance) {
        best = p;
        bestDistance = d;
      }
    }
    return best;
  }
}
