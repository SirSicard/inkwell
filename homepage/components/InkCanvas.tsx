"use client";

import { useEffect, useRef, useState } from "react";

/*
  The ink identity, ported from the app's own audio-reactive shader
  (src/components/InkCanvas.tsx in the Tauri app): simplex-noise blob, cream
  background, charcoal ink, film grain.

  Differences from the app build, on purpose:
  - No audio uniforms. The site has no microphone, so the blob just breathes.
  - Renders inside a contained panel rather than full-bleed, mirroring how the
    app frames the cream ink panel inside its charcoal window.
  - Pauses when scrolled out of view or when the tab is hidden.
  - prefers-reduced-motion: draws exactly one frame and stops.
*/

const VERTEX_SRC = `
attribute vec2 a_position;
void main() {
  gl_Position = vec4(a_position, 0.0, 1.0);
}
`;

const FRAGMENT_SRC = `
precision highp float;
uniform vec2 u_resolution;
uniform float u_time;
uniform vec3 u_bg;
uniform vec3 u_ink;
uniform float u_blobSize;
uniform float u_warp;

vec3 permute(vec3 x) { return mod(((x*34.0)+1.0)*x, 289.0); }

float snoise(vec2 v) {
  const vec4 C = vec4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
  vec2 i = floor(v + dot(v, C.yy));
  vec2 x0 = v - i + dot(i, C.xx);
  vec2 i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
  vec4 x12 = x0.xyxy + C.xxzz;
  x12.xy -= i1;
  i = mod(i, 289.0);
  vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
  vec3 m = max(0.5 - vec3(dot(x0,x0), dot(x12.xy,x12.xy), dot(x12.zw,x12.zw)), 0.0);
  m = m*m; m = m*m;
  vec3 x = 2.0 * fract(p * C.www) - 1.0;
  vec3 h = abs(x) - 0.5;
  vec3 ox = floor(x + 0.5);
  vec3 a0 = x - ox;
  m *= 1.79284291400159 - 0.85373472095314 * (a0*a0 + h*h);
  vec3 g;
  g.x = a0.x * x0.x + h.x * x0.y;
  g.yz = a0.yz * x12.xz + h.yz * x12.yw;
  return 130.0 * dot(m, g);
}

// Two octaves, low frequency, for the shape. More octaves put fine noise on the
// boundary, which reads as facets rather than liquid: surface tension does not
// produce corners.
float fbmSoft(vec2 p) {
  return snoise(p) * 0.68 + snoise(p * 2.0 + 5.2) * 0.32;
}

// Metaball field. Summed inverse-square falloff from several centres, then
// thresholded, so droplets bulge toward each other and merge with a neck the way
// ink does rather than overlapping like discs.
float droplet(vec2 p, vec2 c, float r) {
  vec2 d = p - c;
  return (r * r) / max(dot(d, d), 1e-5);
}

void main() {
  vec2 st = gl_FragCoord.xy / u_resolution.xy;
  float aspect = u_resolution.x / u_resolution.y;
  vec2 pos = st;
  pos.x *= aspect;

  // The single speed dial. Every noise sample below is driven off t, so scaling
  // it here changes the whole animation together and keeps the motion coherent.
  float t = u_time * 0.17325;

  vec2 center = vec2(0.5 * aspect, 0.5);

  // Sizes are in units of the narrow axis. The field is evaluated where y spans
  // 0..1 and x spans 0..aspect, so a radius written as a fraction of height
  // covers far more of the width in a non-square panel.
  float s = min(aspect, 1.0);

  // Domain warp: displace the sampling position before evaluating the field, so
  // the whole mass deforms as a body. Warping the threshold instead, which is
  // what the previous single-blob version did, only ever wobbles an outline.
  vec2 warp = vec2(
    fbmSoft(pos * 1.75 + vec2(t * 0.17, t * 0.10)),
    fbmSoft(pos * 1.75 + vec2(-t * 0.12, t * 0.15) + 19.7)
  );
  vec2 wp = pos + warp * (s * u_warp);

  float field = 0.0;
  field += droplet(wp, center + s * vec2(0.05 * sin(t * 0.9), 0.06 + 0.05 * cos(t * 0.7)), s * u_blobSize);
  field += droplet(wp, center + s * vec2(-0.26 + 0.09 * sin(t * 1.10 + 1.0), -0.30 + 0.08 * cos(t * 0.95)), s * u_blobSize * 0.53);
  field += droplet(wp, center + s * vec2(0.30 + 0.08 * cos(t * 0.85 + 2.0), 0.34 + 0.09 * sin(t * 1.05 + 0.7)), s * u_blobSize * 0.46);
  field += droplet(wp, center + s * vec2(0.22 + 0.07 * sin(t * 1.20 + 3.1), -0.58 + 0.08 * cos(t * 1.00 + 1.7)), s * u_blobSize * 0.35);

  // Tight threshold, with the wicking done by a separate bleed outside it. Ink
  // has a defined boundary that seeps, not an airbrushed falloff.
  float edge = 0.07;
  float blob = smoothstep(1.0 - edge, 1.0 + edge, field);
  float bleed = smoothstep(1.0 - edge * 4.0, 1.0 - edge, field) * (1.0 - blob);

  float detail = fbmSoft(wp * 6.0 + t * 0.25) * 0.02 * blob;
  float grain = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);

  vec3 bgColor = u_bg + grain * 0.02;
  vec3 inkColor = u_ink + detail + grain * 0.015;
  vec3 col = mix(bgColor, inkColor, blob);
  col = mix(col, inkColor, bleed * 0.30);
  gl_FragColor = vec4(col, 1.0);
}
`;

function compile(
  gl: WebGLRenderingContext,
  type: number,
  src: string,
): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, src);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    gl.deleteShader(shader);
    return null;
  }
  return shader;
}

/**
 * "panel" is the app's own inversion: dark ink on a cream field, framed by the
 * charcoal page. "backdrop" is the full-bleed hero treatment: the same motion
 * and grain, but drawn as charcoal-on-charcoal with a faint warm lift, because
 * a cream field behind body copy would blow out the page and leave the text
 * fighting a moving background for contrast. The blob is also larger and softer
 * there, since it is reading as a field rather than an object.
 */
export type InkVariant = "panel" | "backdrop";

const VARIANTS: Record<
  InkVariant,
  { bg: [number, number, number]; ink: [number, number, number]; blobSize: number; warp: number }
> = {
  panel: { bg: [0.94, 0.93, 0.91], ink: [0.06, 0.06, 0.06], blobSize: 0.30, warp: 0.26 },
  // #0e0e11 page, lifted to roughly #4b3c2d in the ink: a warm pigment rather
  // than a grey, and bright enough to actually read as a shape at full window
  // width. Measured contrast of the hero text over this is 9.07:1, still well
  // clear of WCAG AAA (7:1), so the visibility is bought out of headroom rather
  // than out of legibility.
  backdrop: { bg: [0.055, 0.055, 0.067], ink: [0.294, 0.235, 0.176], blobSize: 0.42, warp: 0.30 },
};

export default function InkCanvas({ variant = "panel" }: { variant?: InkVariant } = {}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const gl = canvas.getContext("webgl", { antialias: false, alpha: false });
    if (!gl) {
      setFailed(true);
      return;
    }

    const vs = compile(gl, gl.VERTEX_SHADER, VERTEX_SRC);
    const fs = compile(gl, gl.FRAGMENT_SHADER, FRAGMENT_SRC);
    const program = vs && fs ? gl.createProgram() : null;
    if (!vs || !fs || !program) {
      setFailed(true);
      return;
    }
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      setFailed(true);
      return;
    }

    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([-1, 1, 1, 1, -1, -1, 1, -1]),
      gl.STATIC_DRAW,
    );
    const posLoc = gl.getAttribLocation(program, "a_position");
    gl.enableVertexAttribArray(posLoc);
    gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 0, 0);

    const uRes = gl.getUniformLocation(program, "u_resolution");
    const uTime = gl.getUniformLocation(program, "u_time");
    const uBg = gl.getUniformLocation(program, "u_bg");
    const uInk = gl.getUniformLocation(program, "u_ink");
    const uBlobSize = gl.getUniformLocation(program, "u_blobSize");
    const uWarp = gl.getUniformLocation(program, "u_warp");
    const palette = VARIANTS[variant];

    const start = performance.now();
    let raf = 0;
    let running = false;
    let onScreen = true;

    const draw = (elapsedSeconds: number) => {
      gl.useProgram(program);
      gl.uniform2f(uRes, canvas.width, canvas.height);
      gl.uniform1f(uTime, elapsedSeconds);
      gl.uniform3fv(uBg, palette.bg);
      gl.uniform3fv(uInk, palette.ink);
      gl.uniform1f(uBlobSize, palette.blobSize);
      gl.uniform1f(uWarp, palette.warp);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    };

    const resize = () => {
      // The shader is per-pixel and evaluates three simplex-noise octaves per
      // fragment, so cost scales with the backing-store area. Phones are both
      // the highest-DPR and the least thermally forgiving devices here, so cap
      // them at 1.5x: the blob is a soft gradient with film grain and the
      // difference is invisible, while the fragment count drops by ~44%.
      const cap = window.innerWidth < 640 ? 1.5 : 2;
      const dpr = Math.min(window.devicePixelRatio || 1, cap);
      const rect = canvas.getBoundingClientRect();
      const w = Math.max(1, Math.round(rect.width * dpr));
      const h = Math.max(1, Math.round(rect.height * dpr));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
        gl.viewport(0, 0, w, h);
      }
    };

    const motionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");

    const loop = () => {
      draw(((performance.now() - start) / 1000) % 600);
      raf = requestAnimationFrame(loop);
    };

    const stop = () => {
      if (!running) return;
      running = false;
      cancelAnimationFrame(raf);
    };

    const play = () => {
      resize();
      // Static fallback: one frame of the same shader, no animation loop.
      if (motionQuery.matches) {
        stop();
        draw(0);
        return;
      }
      if (running || !onScreen || document.hidden) return;
      running = true;
      raf = requestAnimationFrame(loop);
    };

    // Only render while the panel is actually on screen.
    const io = new IntersectionObserver(
      ([entry]) => {
        onScreen = entry.isIntersecting;
        if (onScreen) play();
        else stop();
      },
      { threshold: 0 },
    );
    io.observe(canvas);

    // The panel is a grid cell whose height follows the hero's text column, so
    // it changes size without the window ever resizing; swapped webfonts and
    // text reflow both do it. A window-resize listener alone leaves the backing
    // store at its first-paint size and WebGL stretches the blob to fit
    // (measured: a 492x338 box still holding a 488x365 buffer).
    const ro = new ResizeObserver(() => {
      resize();
      if (!running) draw(0);
    });
    ro.observe(canvas);

    // Mobile GPUs drop WebGL contexts under memory pressure or on backgrounding.
    // Without this the canvas keeps its last (now blank) framebuffer and the
    // panel reads as an empty hole; unmounting it lets the wrapper's static
    // radial gradient, the same fallback used when WebGL is unavailable,
    // show through instead.
    const onContextLost = (e: Event) => {
      e.preventDefault();
      stop();
      setFailed(true);
    };

    const onVisibility = () => (document.hidden ? stop() : play());
    const onResize = () => {
      resize();
      if (!running) draw(0);
    };
    const onMotionChange = () => play();

    canvas.addEventListener("webglcontextlost", onContextLost);
    window.addEventListener("resize", onResize);
    document.addEventListener("visibilitychange", onVisibility);
    motionQuery.addEventListener("change", onMotionChange);

    play();

    return () => {
      stop();
      io.disconnect();
      ro.disconnect();
      canvas.removeEventListener("webglcontextlost", onContextLost);
      // Kept alongside the ResizeObserver: a browser-zoom or monitor change
      // moves devicePixelRatio without changing the element's CSS box.
      window.removeEventListener("resize", onResize);
      document.removeEventListener("visibilitychange", onVisibility);
      motionQuery.removeEventListener("change", onMotionChange);
      gl.deleteProgram(program);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      gl.deleteBuffer(buf);
    };
  }, [variant]);

  return (
    <div
      aria-hidden="true"
      className="absolute inset-0 overflow-hidden rounded-[inherit]"
      style={{
        // Doubles as the no-WebGL fallback, per variant: the panel keeps the
        // cream field and dark core; the backdrop stays on the page colour so a
        // WebGL failure degrades to a plain dark hero rather than a bright slab
        // with unreadable text over it.
        background:
          variant === "backdrop"
            ? "radial-gradient(circle at 50% 45%, #221c17 0%, #16151a 45%, #0e0e11 100%)"
            : "radial-gradient(circle at 50% 50%, #101010 0%, #151515 30%, #efeee9 32%, #f3f1ec 100%)",
      }}
    >
      {!failed && <canvas ref={canvasRef} className="block h-full w-full" />}
    </div>
  );
}
