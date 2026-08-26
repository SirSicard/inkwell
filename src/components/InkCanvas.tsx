import { useEffect, useRef } from "react"
import { listen } from "@tauri-apps/api/event"

const vertexShader = `
  attribute vec4 aPosition;
  void main() {
    gl_Position = aPosition;
  }
`

const fragmentShader = `
  precision highp float;
  uniform vec2 u_resolution;
  uniform float u_time;
  uniform float u_amplitude;
  uniform float u_low;    // 80-300Hz: bass, voice body
  uniform float u_mid;    // 300-2kHz: voice presence, consonants
  uniform float u_high;   // 2k-8kHz: sibilance, detail
  uniform float u_state;  // 0 = idle, 1 = recording (spring-interpolated)
  uniform sampler2D u_text; // wordmark alpha, composited as a true inversion

  vec3 permute(vec3 x) { return mod(((x*34.0)+1.0)*x, 289.0); }

  float snoise(vec2 v) {
    const vec4 C = vec4(0.211324865405187, 0.366025403784439,
                       -0.577350269189626, 0.024390243902439);
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

  // Fractal noise: several octaves of simplex. One octave gives the smooth,
  // faintly circular wobble the old blob had; stacking them is what produces an
  // edge that reads as liquid rather than as a warped circle.
  float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    for (int i = 0; i < 4; i++) {
      v += a * snoise(p);
      p *= 2.03;
      a *= 0.5;
    }
    return v;
  }

  // Two octaves, low frequency, for the shape itself. Four octaves warp the
  // boundary at fine scales, which is what made the mass read as faceted rather
  // than liquid: surface tension does not produce that. The full fbm above is
  // kept for interior texture, where fine noise is welcome.
  float fbmSoft(vec2 p) {
    return snoise(p) * 0.68 + snoise(p * 2.0 + 5.2) * 0.32;
  }

  // Metaball field. Summing inverse-square falloff from several centres and
  // thresholding the total is what makes droplets bulge toward each other and
  // merge with a neck, the way ink does, instead of overlapping like discs.
  float droplet(vec2 p, vec2 c, float r) {
    vec2 d = p - c;
    return (r * r) / max(dot(d, d), 1e-5);
  }

  void main() {
    vec2 st = gl_FragCoord.xy / u_resolution.xy;
    float aspect = u_resolution.x / u_resolution.y;
    vec2 pos = st;
    pos.x *= aspect;

    float amp = u_amplitude;
    float stateBoost = u_state * 0.3;
    // Roughly double the previous rate. The composition is smaller now, and a
    // small shape drifting at the speed of a large one reads as stalled.
    float t = u_time * (0.32 + amp * 0.35 + u_low * 0.12 + stateBoost * 0.25);

    vec2 center = vec2(0.5 * aspect, 0.5);

    // Everything below is expressed in units of the panel's NARROW axis. The
    // field is evaluated in a space where y spans 0..1 and x spans 0..aspect, so
    // a radius written as a fraction of height covers most of the width in a
    // tall column like this one, which is why the first pass filled the panel
    // with a single mass instead of reading as droplets.
    float s = min(aspect, 1.0);

    // Domain warp: displace the sampling position before evaluating the field,
    // so the whole mass deforms as one body. Warping the threshold instead,
    // which is what this used to do, only ever wobbles the outline.
    vec2 warp = vec2(
      fbmSoft(pos * 1.75 + vec2(t * 0.17, t * 0.10)),
      fbmSoft(pos * 1.75 + vec2(-t * 0.12, t * 0.15) + 19.7)
    );
    // Frequency and amount are a pair, and both extremes fail. Four octaves at
    // 2.1 faceted the edge; two octaves at 1.15 mostly translated the shape and
    // left a smooth egg. Mid frequency with a larger amount deforms the whole
    // body into something irregular while keeping the curvature liquid.
    float warpAmt = s * (0.26 + amp * 0.22 + u_mid * 0.12 + stateBoost * 0.10);
    vec2 wp = pos + warp * warpAmt;

    // A main mass plus three satellites, at different sizes and drift rates.
    // The offsets are deliberately uneven: evenly spaced droplets read as a
    // pattern, uneven ones read as a spill.
    float pulse = u_low * 0.10 + amp * 0.06 + stateBoost * 0.05;

    float field = 0.0;
    // Sized to leave cream on every side. The mass plus its warp has to stay
    // inside the panel, or it stops reading as a spill on paper and starts
    // reading as a dark background with a light strip down one edge.
    // Main mass, a little below centre.
    field += droplet(wp, center + s * vec2(0.05 * sin(t * 0.9), 0.06 + 0.05 * cos(t * 0.7)), s * (0.30 + pulse));
    // Satellites: the first stays close enough to hold a neck to the body, the
    // others separate and rejoin as the warp moves past them.
    field += droplet(wp, center + s * vec2(-0.26 + 0.09 * sin(t * 1.10 + 1.0), -0.30 + 0.08 * cos(t * 0.95)), s * (0.16 + pulse * 0.6));
    field += droplet(wp, center + s * vec2(0.30 + 0.08 * cos(t * 0.85 + 2.0), 0.34 + 0.09 * sin(t * 1.05 + 0.7)), s * (0.14 + pulse * 0.5));
    // Was 0.085, which at this scale rendered as a speck rather than a droplet.
    field += droplet(wp, center + s * vec2(0.22 + 0.07 * sin(t * 1.20 + 3.1), -0.58 + 0.08 * cos(t * 1.00 + 1.7)), s * (0.105 + pulse * 0.4));

    // A tight threshold keeps the edge crisp. Ink has a defined boundary with a
    // little bleed, not an airbrushed falloff, and the old wide smoothstep over
    // a plain distance field was most of why it read as a smudge.
    float edge = 0.07 + u_high * 0.02;
    float blob = smoothstep(1.0 - edge, 1.0 + edge, field);

    // Feathered bleed just outside the body, like ink wicking into paper.
    float bleed = smoothstep(1.0 - edge * 4.0, 1.0 - edge, field) * (1.0 - blob);

    float detail = fbm(wp * 6.0 + t * 0.25) * (0.020 + u_high * 0.05 + stateBoost * 0.03) * blob;
    float grain = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);

    vec3 bgColor = vec3(0.94, 0.93, 0.91) + grain * 0.02;
    vec3 inkColor = vec3(0.06 + detail) + grain * 0.015;

    vec3 finalColor = mix(bgColor, inkColor, blob);
    finalColor = mix(finalColor, inkColor, bleed * 0.30);

    // The wordmark, composited here rather than as a DOM element above the
    // canvas. It was a <span> with mix-blend-difference, which inverts correctly
    // in Chromium but not in the WKWebView this app ships in: a WebGL canvas is
    // a hardware-composited layer, and WebKit will not reliably blend DOM
    // against one, so the letters stayed white over the cream field instead of
    // turning dark. Inverting in the shader is exact and engine-independent.
    float mark = texture2D(u_text, vec2(st.x, 1.0 - st.y)).a;
    finalColor = mix(finalColor, 1.0 - finalColor, mark);

    gl_FragColor = vec4(finalColor, 1.0);
  }
`

// Frequency band extraction from AnalyserNode data
function extractBands(dataArray: Uint8Array, sampleRate: number, fftSize: number) {
  const binHz = sampleRate / fftSize
  // Band boundaries in bin indices
  const lowStart = Math.floor(80 / binHz)
  const lowEnd = Math.floor(300 / binHz)
  const midStart = lowEnd
  const midEnd = Math.floor(2000 / binHz)
  const highStart = midEnd
  const highEnd = Math.min(Math.floor(8000 / binHz), dataArray.length)

  let lowSum = 0, midSum = 0, highSum = 0, totalSum = 0

  for (let i = lowStart; i < lowEnd && i < dataArray.length; i++) {
    lowSum += dataArray[i]
  }
  for (let i = midStart; i < midEnd && i < dataArray.length; i++) {
    midSum += dataArray[i]
  }
  for (let i = highStart; i < highEnd && i < dataArray.length; i++) {
    highSum += dataArray[i]
  }
  for (let i = 0; i < dataArray.length; i++) {
    totalSum += dataArray[i]
  }

  const lowCount = Math.max(lowEnd - lowStart, 1)
  const midCount = Math.max(midEnd - midStart, 1)
  const highCount = Math.max(highEnd - highStart, 1)

  return {
    low: (lowSum / lowCount) / 255,
    mid: (midSum / midCount) / 255,
    high: (highSum / highCount) / 255,
    rms: (totalSum / dataArray.length) / 255,
  }
}

export function InkCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  // Filled by the effect that starts the animation, not here. Reading the
  // clock during render is impure, and the honest place to start an animation
  // clock is where the animation starts: a component that renders and then
  // never gets a GL context should not be told it has been running.
  const startTimeRef = useRef<number | null>(null)
  const rafRef = useRef<number>(0)

  // Audio data refs
  const amplitudeRef = useRef(0)
  const lowRef = useRef(0)
  const midRef = useRef(0)
  const highRef = useRef(0)

  // Recording state: 0 = idle, 1 = recording
  const stateRef = useRef(0)

  // Smoothed values for the shader
  const smoothAmpRef = useRef(0)
  const smoothLowRef = useRef(0)
  const smoothMidRef = useRef(0)
  const smoothHighRef = useRef(0)
  const smoothStateRef = useRef(0)

  // Listen for recording state changes from Rust backend
  const analyserRef = useRef<AnalyserNode | null>(null)
  // Assigned by the analysis effect below; called from the recording listener.
  const startAnalysisRef = useRef<() => void>(() => {})
  const stopAnalysisRef = useRef<() => void>(() => {})
  useEffect(() => {
    const unlisten = listen<boolean>("recording-state", (event) => {
      const wasRecording = stateRef.current === 1
      stateRef.current = event.payload ? 1 : 0

      // Reset audio data on recording start to prevent stale buffer burst
      if (event.payload && !wasRecording) {
        amplitudeRef.current = 0
        lowRef.current = 0
        midRef.current = 0
        highRef.current = 0
        smoothAmpRef.current = 0
        smoothLowRef.current = 0
        smoothMidRef.current = 0
        smoothHighRef.current = 0
        startAnalysisRef.current()
      } else if (!event.payload && wasRecording) {
        // Zeroing the targets lets the render loop's lerp decay the blob to
        // rest, which the analyser pump used to do frame by frame.
        stopAnalysisRef.current()
        amplitudeRef.current = 0
        lowRef.current = 0
        midRef.current = 0
        highRef.current = 0
      }
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  // Fallback: listen for Tauri audio-amplitude events (from Rust backend)
  useEffect(() => {
    const unlisten = listen<number>("audio-amplitude", (event) => {
      amplitudeRef.current = Math.min(event.payload * 3, 0.5)
    })
    return () => { unlisten.then((fn) => fn()) }
  }, [])

  // Frequency analysis for the shader, via the WebView's own capture.
  //
  // This is a second microphone stream alongside Rust's cpal, and it used to be
  // opened at mount and held for the life of the process, which parked a
  // permanent orange mic indicator in the macOS menu bar for an app whose whole
  // promise is that it is not listening to you. It now opens on recording start
  // and is fully torn down on stop, so the indicator means what it says.
  //
  // Rust's audio-amplitude events (above) still drive the base reaction, so the
  // ~100ms while getUserMedia resolves is covered; the analyser only adds the
  // low/mid/high split once it is live.
  useEffect(() => {
    let disposed = false
    let ctx: AudioContext | null = null
    let stream: MediaStream | null = null
    let raf = 0

    const stop = () => {
      if (raf) cancelAnimationFrame(raf)
      raf = 0
      analyserRef.current = null
      stream?.getTracks().forEach((t) => t.stop())
      stream = null
      ctx?.close().catch(() => {})
      ctx = null
    }

    const start = async () => {
      if (ctx || stream || disposed) return
      try {
        stream = await navigator.mediaDevices.getUserMedia({ audio: true })
        // Recording can end while the permission round-trip is in flight; if it
        // did, release the device rather than leaving it hot.
        if (disposed || stateRef.current !== 1) {
          stream.getTracks().forEach((t) => t.stop())
          stream = null
          return
        }

        ctx = new AudioContext()
        const analyser = ctx.createAnalyser()
        analyser.fftSize = 2048
        analyser.smoothingTimeConstant = 0.6
        ctx.createMediaStreamSource(stream).connect(analyser)
        analyserRef.current = analyser

        const data = new Uint8Array(analyser.frequencyBinCount)
        const sampleRate = ctx.sampleRate

        const pump = () => {
          if (analyserRef.current !== analyser) return
          analyser.getByteFrequencyData(data as Uint8Array<ArrayBuffer>)
          const bands = extractBands(data, sampleRate, analyser.fftSize)
          // Scale down and clamp to keep ink movement controlled
          // Dialed back from 0.5 (too reactive, 1.6.6 feedback)
          amplitudeRef.current = Math.min(bands.rms * 0.3, 0.35)
          lowRef.current = Math.min(bands.low * 0.3, 0.35)
          midRef.current = Math.min(bands.mid * 0.3, 0.35)
          highRef.current = Math.min(bands.high * 0.3, 0.35)
          raf = requestAnimationFrame(pump)
        }
        pump()
      } catch (err) {
        // No mic permission in the webview: the Rust amplitude events still
        // drive the blob, just without the frequency split.
        console.warn("Web Audio unavailable, using Tauri amplitude events:", err)
        stop()
      }
    }

    startAnalysisRef.current = () => { void start() }
    stopAnalysisRef.current = stop

    return () => {
      disposed = true
      stop()
    }
  }, [])

  // WebGL shader
  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return

    const gl = canvas.getContext("webgl", { antialias: false, alpha: false })
    if (!gl) return

    const vs = gl.createShader(gl.VERTEX_SHADER)!
    gl.shaderSource(vs, vertexShader)
    gl.compileShader(vs)

    const fs = gl.createShader(gl.FRAGMENT_SHADER)!
    gl.shaderSource(fs, fragmentShader)
    gl.compileShader(fs)

    if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
      console.error("Fragment shader error:", gl.getShaderInfoLog(fs))
    }

    const program = gl.createProgram()!
    gl.attachShader(program, vs)
    gl.attachShader(program, fs)
    gl.linkProgram(program)

    const buffer = gl.createBuffer()
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, 1, 1, 1, -1, -1, 1, -1]), gl.STATIC_DRAW)

    const posLoc = gl.getAttribLocation(program, "aPosition")
    gl.enableVertexAttribArray(posLoc)
    gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 0, 0)

    const textLoc = gl.getUniformLocation(program, "u_text")

    // The wordmark as a texture. Rendered to an offscreen 2D canvas at the
    // panel's pixel size, uploaded once, and re-rendered on resize. Waits for
    // document.fonts.ready: without it the first upload can land in a fallback
    // face and then never update, since this is not redrawn per frame.
    const textTex = gl.createTexture()
    const textCanvas = document.createElement("canvas")
    let textReady = false

    const drawWordmark = () => {
      const w = canvas.width
      const h = canvas.height
      if (w < 2 || h < 2) return
      textCanvas.width = w
      textCanvas.height = h
      const c = textCanvas.getContext("2d")
      if (!c) return

      c.clearRect(0, 0, w, h)
      // Scale with the panel so the mark keeps its proportion at any width, and
      // cap it so it cannot overflow a narrow window.
      const size = Math.min(w * 0.17, h * 0.11)
      c.font = `900 ${size}px "Geist Sans", system-ui, sans-serif`
      c.textAlign = "center"
      c.textBaseline = "top"
      c.fillStyle = "#fff"
      // Alpha is all the shader reads; the colour here is irrelevant.
      c.fillText("INKWELL", w / 2, h * 0.045)

      gl.bindTexture(gl.TEXTURE_2D, textTex)
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, 0)
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, textCanvas)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
      textReady = true
    }

    if (document.fonts && document.fonts.ready) {
      document.fonts.ready.then(drawWordmark).catch(drawWordmark)
    } else {
      drawWordmark()
    }

    const resize = () => {
      const rect = canvas.parentElement!.getBoundingClientRect()
      const dpr = Math.min(window.devicePixelRatio || 1, 2)
      canvas.width = rect.width * dpr
      canvas.height = rect.height * dpr
      canvas.style.width = rect.width + "px"
      canvas.style.height = rect.height + "px"
      gl.viewport(0, 0, canvas.width, canvas.height)
      drawWordmark()
    }
    resize()
    window.addEventListener("resize", resize)

    const resLoc = gl.getUniformLocation(program, "u_resolution")
    const timeLoc = gl.getUniformLocation(program, "u_time")
    const ampLoc = gl.getUniformLocation(program, "u_amplitude")
    const lowLoc = gl.getUniformLocation(program, "u_low")
    const midLoc = gl.getUniformLocation(program, "u_mid")
    const highLoc = gl.getUniformLocation(program, "u_high")
    const stateLoc = gl.getUniformLocation(program, "u_state")


    startTimeRef.current = Date.now()

    const lerpFactor = 0.05  // Smoothing speed (lower = smoother, less twitchy)
    const stateLerpFactor = 0.04 // Slower spring for state transitions

    const render = () => {
      // Wrap time to prevent float precision issues at large values.
      // `?? Date.now()` is for the type only: the effect above sets this
      // before it ever schedules a frame, so the fallback means "zero elapsed".
      const elapsed = ((Date.now() - (startTimeRef.current ?? Date.now())) / 1000) % 600

      // Smooth all values with hard clamp
      smoothAmpRef.current += (amplitudeRef.current - smoothAmpRef.current) * lerpFactor
      smoothLowRef.current += (lowRef.current - smoothLowRef.current) * lerpFactor
      smoothMidRef.current += (midRef.current - smoothMidRef.current) * lerpFactor
      smoothHighRef.current += (highRef.current - smoothHighRef.current) * lerpFactor
      smoothStateRef.current += (stateRef.current - smoothStateRef.current) * stateLerpFactor

      // Hard clamp smoothed values
      smoothAmpRef.current = Math.min(smoothAmpRef.current, 0.45)
      smoothLowRef.current = Math.min(smoothLowRef.current, 0.45)
      smoothMidRef.current = Math.min(smoothMidRef.current, 0.45)
      smoothHighRef.current = Math.min(smoothHighRef.current, 0.45)

      gl.useProgram(program)
      gl.uniform2f(resLoc, canvas.width, canvas.height)
      gl.uniform1f(timeLoc, elapsed)
      gl.uniform1f(ampLoc, smoothAmpRef.current)
      gl.uniform1f(lowLoc, smoothLowRef.current)
      gl.uniform1f(midLoc, smoothMidRef.current)
      gl.uniform1f(highLoc, smoothHighRef.current)
      gl.uniform1f(stateLoc, smoothStateRef.current)
      if (textReady) {
        gl.activeTexture(gl.TEXTURE0)
        gl.bindTexture(gl.TEXTURE_2D, textTex)
        gl.uniform1i(textLoc, 0)
      }
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4)

      rafRef.current = requestAnimationFrame(render)
    }
    rafRef.current = requestAnimationFrame(render)

    return () => {
      cancelAnimationFrame(rafRef.current)
      window.removeEventListener("resize", resize)
    }
  }, [])

  return (
    <canvas
      ref={canvasRef}
      className="w-full h-full block"
    />
  )
}
