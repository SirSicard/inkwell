/*
  Licence notice: the shader's snoise() is 2D simplex noise from webgl-noise (Ian McEwan, Ashima Arts;
  https://github.com/ashima/webgl-noise, maintained at https://github.com/stegu/webgl-noise), MIT
  licensed. It reached this file through the Inkwell app's own InkCanvas shader. The full MIT notice
  sits at the top of the FRAG string below, as a GLSL comment, so it ships inside the built JavaScript
  with the code it covers (a JS comment would be stripped by the minifier).
*/

/*
  <ink-field>: the Inkwell ink identity as a framework-free custom element.

  The shader is the app's own (simplex-noise metaballs, domain warp, a bleed edge, film grain),
  ported from homepage/components/InkCanvas.tsx. What changed, and why:
  - `amp` (0..1) is a live input. The app drives it from the microphone; the site drives it from
    a demo interaction (hold a key, press the button) or leaves it breathing at 0.
  - The element's own CSS background is the poster. The canvas is only added after WebGL has
    drawn its first frame, so a failure, a zero-size box or a lost context leaves the poster.
  - Canvas guard: nothing renders while the element has no size. It retries from ResizeObserver.
  - Reduced motion draws one still frame and never loops. Off-screen or a hidden tab pauses.

  Attributes: bg, ink (hex), size (blob radius, 0.2-0.5), warp (0.1-0.4), speed (1 = the app's pace).

  Additions for the homepage's hotkey demo (the guard, the poster, the reduced-motion still and the
  pauses are unchanged):
  - `rest` attribute: the ink is still unless a level drives it, then settles and stops drawing. The
    instrument's indicator does not fidget, and it is the app's own rule: draw nothing when idle.
    Motion only ever lasts as long as a (demo) voice does, so nothing moves on its own for > 5 s.
  - setLevel(v): drive `amp` from outside. Under reduced motion it redraws one still frame at that
    level (an instant state change, not movement).
  - The WebGL context is not even requested until the element has a non-zero box.
  - Easing is time-based, so 120 Hz screens do not swell twice as fast.

  Build step: the model-meter stills (inkStill) were removed with the meters. The ink now only ever
  stands for a voice.
*/

const VERT = `attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}`;

const FRAG = `
// snoise(): webgl-noise, https://github.com/ashima/webgl-noise (Ian McEwan, Ashima Arts)
// Copyright (C) 2011 by Ashima Arts (Simplex noise)
// Copyright (C) 2011-2016 by Stefan Gustavson (Classic noise and others)
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
// associated documentation files (the "Software"), to deal in the Software without restriction,
// including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so,
// subject to the following conditions:
// The above copyright notice and this permission notice shall be included in all copies or substantial
// portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT
// LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
// NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
// WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
// SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
precision highp float;
uniform vec2 u_res; uniform float u_time; uniform vec3 u_bg; uniform vec3 u_ink;
uniform float u_size; uniform float u_warp; uniform float u_amp;
uniform float u_sat; uniform float u_spread;
vec3 permute(vec3 x){return mod(((x*34.0)+1.0)*x,289.0);}
float snoise(vec2 v){
  const vec4 C=vec4(0.211324865405187,0.366025403784439,-0.577350269189626,0.024390243902439);
  vec2 i=floor(v+dot(v,C.yy)); vec2 x0=v-i+dot(i,C.xx);
  vec2 i1=(x0.x>x0.y)?vec2(1.0,0.0):vec2(0.0,1.0);
  vec4 x12=x0.xyxy+C.xxzz; x12.xy-=i1; i=mod(i,289.0);
  vec3 p=permute(permute(i.y+vec3(0.0,i1.y,1.0))+i.x+vec3(0.0,i1.x,1.0));
  vec3 m=max(0.5-vec3(dot(x0,x0),dot(x12.xy,x12.xy),dot(x12.zw,x12.zw)),0.0); m=m*m; m=m*m;
  vec3 x=2.0*fract(p*C.www)-1.0; vec3 h=abs(x)-0.5; vec3 ox=floor(x+0.5); vec3 a0=x-ox;
  m*=1.79284291400159-0.85373472095314*(a0*a0+h*h);
  vec3 g; g.x=a0.x*x0.x+h.x*x0.y; g.yz=a0.yz*x12.xz+h.yz*x12.yw;
  return 130.0*dot(m,g);
}
float fbm(vec2 p){return snoise(p)*0.68+snoise(p*2.0+5.2)*0.32;}
float drop(vec2 p,vec2 c,float r){vec2 d=p-c;return (r*r)/max(dot(d,d),1e-5);}
void main(){
  vec2 st=gl_FragCoord.xy/u_res.xy; float aspect=u_res.x/u_res.y; vec2 pos=st; pos.x*=aspect;
  float t=u_time*0.17325; vec2 c=vec2(0.5*aspect,0.5); float s=min(aspect,1.0);
  float warpAmt=u_warp*(1.0+u_amp*0.9);
  vec2 w=vec2(fbm(pos*1.75+vec2(t*0.17,t*0.10)),fbm(pos*1.75+vec2(-t*0.12,t*0.15)+19.7));
  vec2 wp=pos+w*(s*warpAmt);
  float r=u_size*(1.0+u_amp*0.35);
  float k=s*u_spread; float q=s*r*u_sat;
  float f=0.0;
  f+=drop(wp,c+s*vec2(0.05*sin(t*0.9),0.06+0.05*cos(t*0.7)),s*r);
  f+=drop(wp,c+k*vec2(-0.26+0.09*sin(t*1.10+1.0),-0.30+0.08*cos(t*0.95)),q*0.53);
  f+=drop(wp,c+k*vec2(0.30+0.08*cos(t*0.85+2.0),0.34+0.09*sin(t*1.05+0.7)),q*0.46);
  f+=drop(wp,c+k*vec2(0.22+0.07*sin(t*1.20+3.1),-0.58+0.08*cos(t*1.00+1.7)),q*0.35);
  float e=0.07; float blob=smoothstep(1.0-e,1.0+e,f);
  float bleed=smoothstep(1.0-e*4.0,1.0-e,f)*(1.0-blob);
  float detail=fbm(wp*6.0+t*0.25)*0.02*blob;
  float grain=fract(sin(dot(gl_FragCoord.xy,vec2(12.9898,78.233)))*43758.5453);
  vec3 bg=u_bg+grain*0.02; vec3 ink=u_ink+detail+grain*0.015;
  vec3 col=mix(bg,ink,blob); col=mix(col,ink,bleed*0.30);
  gl_FragColor=vec4(col,1.0);
}`;

/** Frame time used for every still: the reduced-motion frame and the resting frame. */
const STILL = 3.2;

type RGB = [number, number, number];
type Program = {
  prog: WebGLProgram;
  buf: WebGLBuffer | null;
  U: Record<'res' | 'time' | 'bg' | 'ink' | 'size' | 'warp' | 'amp' | 'sat' | 'spread', WebGLUniformLocation | null>;
};
type Look = { bg: RGB; ink: RGB; size: number; warp: number; amp: number; sat: number; spread: number };

function hex(h: string | null | undefined, fallback: RGB): RGB {
  const m = h && /^#?([0-9a-f]{6})$/i.exec(h.trim());
  if (!m) return fallback;
  const n = parseInt(m[1], 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

const num = (v: string | null | undefined, fallback: number) => {
  const n = Number(v);
  return v != null && v !== '' && Number.isFinite(n) ? n : fallback;
};

function compile(gl: WebGLRenderingContext): Program | null {
  const sh = (type: number, src: string) => {
    const s = gl.createShader(type);
    if (!s) return null;
    gl.shaderSource(s, src);
    gl.compileShader(s);
    return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null;
  };
  const vs = sh(gl.VERTEX_SHADER, VERT);
  const fs = sh(gl.FRAGMENT_SHADER, FRAG);
  const prog = vs && fs ? gl.createProgram() : null;
  if (!vs || !fs || !prog) return null;
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return null;

  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, 1, 1, 1, -1, -1, 1, -1]), gl.STATIC_DRAW);
  const loc = gl.getAttribLocation(prog, 'p');
  gl.enableVertexAttribArray(loc);
  gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
  const u = (n: string) => gl.getUniformLocation(prog, n);
  return {
    prog,
    buf,
    U: { res: u('u_res'), time: u('u_time'), bg: u('u_bg'), ink: u('u_ink'), size: u('u_size'), warp: u('u_warp'), amp: u('u_amp'), sat: u('u_sat'), spread: u('u_spread') },
  };
}

function paint(gl: WebGLRenderingContext, P: Program, w: number, h: number, seconds: number, look: Look) {
  gl.viewport(0, 0, w, h);
  gl.useProgram(P.prog);
  gl.uniform2f(P.U.res, w, h);
  gl.uniform1f(P.U.time, seconds);
  gl.uniform3fv(P.U.bg, look.bg);
  gl.uniform3fv(P.U.ink, look.ink);
  gl.uniform1f(P.U.size, look.size);
  gl.uniform1f(P.U.warp, look.warp);
  gl.uniform1f(P.U.amp, look.amp);
  gl.uniform1f(P.U.sat, look.sat);
  gl.uniform1f(P.U.spread, look.spread);
  gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
}

export class InkField extends HTMLElement {
  amp = 0; // 0..1, eased toward target each frame
  target = 0;
  private cleanup: (() => void) | null = null;
  private wake: (() => void) | null = null;
  private waitRO: ResizeObserver | null = null;
  /** A scheduled idle boot, so a disconnect can cancel it and a second connect can't double it. */
  private pending: (() => void) | null = null;

  connectedCallback() {
    if (this.cleanup || this.pending || this.waitRO) return; // already running or on its way
    // The screenshot underneath is the poster and the LCP image. Compiling the shader and drawing the
    // first frame waits until the browser is idle, so it never delays that paint (Lighthouse measured a
    // 0.9 s long task here on a throttled, GPU-less phone profile when it ran straight away). A level
    // arriving first (the hotkey demo) boots it at once instead: see setLevel().
    const run = () => {
      this.pending = null;
      if (this.isConnected) this.attach();
    };
    if (typeof window.requestIdleCallback === 'function') {
      const id = window.requestIdleCallback(run, { timeout: 2000 });
      this.pending = () => window.cancelIdleCallback(id);
    } else {
      const id = window.setTimeout(run, 200);
      this.pending = () => window.clearTimeout(id);
    }
  }

  private attach() {
    const r = this.getBoundingClientRect();
    if (r.width >= 1 && r.height >= 1) {
      this.boot();
      return;
    }
    // Canvas guard: no box yet, so no context yet. Start once the box has a size.
    try {
      this.waitRO = new ResizeObserver((entries) => {
        const box = entries[0]?.contentRect;
        if (!box || box.width < 1 || box.height < 1) return;
        this.waitRO?.disconnect();
        this.waitRO = null;
        this.boot();
      });
      this.waitRO.observe(this);
    } catch {
      this.waitRO = null; // poster stays
    }
  }

  disconnectedCallback() {
    this.pending?.();
    this.pending = null;
    this.waitRO?.disconnect();
    this.waitRO = null;
    this.cleanup?.();
    this.cleanup = null;
    this.wake = null;
  }

  /** Drive the ink from outside (0..1). The hotkey demo calls this with a voice envelope. */
  setLevel(v: number) {
    this.target = Math.max(0, Math.min(1, Number.isFinite(v) ? v : 0));
    if (this.pending && this.target > 0) {
      // A voice has started before the idle boot: start now, so the moment isn't lost.
      this.pending();
      this.pending = null;
      this.attach();
    }
    this.wake?.();
  }

  private boot() {
    if (this.cleanup) return; // never two canvases, loops or sets of observers
    try {
      this.cleanup = this.start();
    } catch {
      this.cleanup = null; // poster stays
    }
  }

  private start(): (() => void) | null {
    const canvas = document.createElement('canvas');
    canvas.setAttribute('aria-hidden', 'true');
    canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;opacity:0;transition:opacity .6s ease';

    const sizeFor = () => {
      const r = this.getBoundingClientRect();
      if (r.width < 1 || r.height < 1) return null; // canvas guard: no size, no draw
      const dpr = Math.min(window.devicePixelRatio || 1, window.innerWidth < 640 ? 1.5 : 2);
      return [Math.max(1, Math.round(r.width * dpr)), Math.max(1, Math.round(r.height * dpr))] as const;
    };
    const first = sizeFor();
    if (!first) return null;
    [canvas.width, canvas.height] = first; // sized before any context exists

    const gl = canvas.getContext('webgl', { antialias: false, alpha: false, preserveDrawingBuffer: false });
    if (!gl) return null;
    const P = compile(gl);
    if (!P) return null;

    const look: Look = {
      bg: hex(this.getAttribute('bg'), [0.94, 0.93, 0.91]),
      ink: hex(this.getAttribute('ink'), [0.06, 0.06, 0.06]),
      size: num(this.getAttribute('size'), 0.3),
      warp: num(this.getAttribute('warp'), 0.26),
      amp: 0,
      sat: 1,
      spread: 1,
    };
    const speed = num(this.getAttribute('speed'), 1);
    const rest = this.hasAttribute('rest');

    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');
    let clock = rest ? STILL : 0;
    let last = 0;
    let raf = 0;
    let running = false;
    let onScreen = true;
    let shown = false;

    const sized = () => {
      const s = sizeFor();
      if (!s) return false;
      if (canvas.width !== s[0] || canvas.height !== s[1]) [canvas.width, canvas.height] = s;
      return true;
    };
    const settled = () => this.target === 0 && this.amp < 0.003;

    const draw = (seconds: number, dt: number) => {
      if (!sized() || gl.isContextLost()) return;
      if (dt > 0) this.amp += (this.target - this.amp) * (1 - Math.exp(-dt / 0.09));
      look.amp = this.amp;
      paint(gl, P, canvas.width, canvas.height, seconds, look);
      if (!shown) {
        shown = true;
        canvas.style.opacity = '1'; // fade in only after a real frame
      }
    };

    const loop = (now: number) => {
      const dt = Math.min(0.1, Math.max(0, (now - last) / 1000));
      last = now;
      clock = (clock + dt * speed) % 600;
      draw(clock, dt);
      if (rest && settled()) {
        running = false; // at rest: the last frame stays, nothing more is drawn
        return;
      }
      raf = requestAnimationFrame(loop);
    };
    const stop = () => {
      running = false;
      cancelAnimationFrame(raf);
    };
    const play = () => {
      if (reduce.matches) {
        stop();
        this.amp = this.target; // an instant state change, never a tween
        draw(STILL, 0); // one still frame
        return;
      }
      if (running || !onScreen || document.hidden) return;
      if (rest && settled()) {
        draw(clock, 0);
        return;
      }
      running = true;
      last = performance.now();
      raf = requestAnimationFrame(loop);
    };
    this.wake = play;

    // Every constructor that can throw runs before anything is attached, so a failure leaves nothing
    // behind (boot()'s catch keeps the poster).
    const io = new IntersectionObserver(([e]) => {
      onScreen = e.isIntersecting;
      if (onScreen) play();
      else stop();
    });
    const ro = new ResizeObserver(() => {
      if (!running) draw(reduce.matches ? STILL : clock, 0);
    });
    this.append(canvas);
    io.observe(this);
    ro.observe(this);
    const onVis = () => (document.hidden ? stop() : play());
    const onLost = (ev: Event) => {
      ev.preventDefault();
      stop();
      this.wake = null;
      canvas.remove(); // poster shows through
    };
    document.addEventListener('visibilitychange', onVis);
    canvas.addEventListener('webglcontextlost', onLost);
    reduce.addEventListener('change', play);
    play();

    return () => {
      stop();
      io.disconnect();
      ro.disconnect();
      document.removeEventListener('visibilitychange', onVis);
      canvas.removeEventListener('webglcontextlost', onLost);
      reduce.removeEventListener('change', play);
      gl.deleteProgram(P.prog);
      gl.deleteBuffer(P.buf);
      canvas.remove();
    };
  }
}

if (!customElements.get('ink-field')) customElements.define('ink-field', InkField);
