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

  void main() {
    vec2 st = gl_FragCoord.xy / u_resolution.xy;
    vec2 pos = st;
    pos.x *= u_resolution.x / u_resolution.y;

    // Recording state boosts all parameters
    float amp = u_amplitude;
    float stateBoost = u_state * 0.3;

    // Time speed: idle = slow, recording = faster (even without audio)
    float t = u_time * (0.15 + amp * 0.25 + u_low * 0.1 + stateBoost * 0.2);

    vec2 center = vec2(0.5 * (u_resolution.x / u_resolution.y), 0.5);
    float dist = length(pos - center);

    // Warp: recording state adds baseline distortion
    float warpIntensity = 0.20 + amp * 0.3 + u_mid * 0.2 + stateBoost * 0.15;
    float n1 = snoise(pos * 2.0 - vec2(t * 0.2, -t * 0.1));
    float n2 = snoise(pos * 4.0 + vec2(t * 0.1, t * 0.2));
    float warp = n1 * warpIntensity + n2 * (warpIntensity * 0.5);

    // Blob size: recording state makes it expand
    float blobSize = 0.38 + u_low * 0.12 + amp * 0.08 + stateBoost * 0.06;
    float blob = smoothstep(blobSize + 0.02, blobSize - 0.02, dist + warp);

    // Surface detail
    float n3 = snoise(pos * 8.0 + vec2(t * 0.5, -t * 0.4));
    float detail = n3 * (0.02 + u_high * 0.06 + stateBoost * 0.03) * blob;

    // Film grain
    float grain = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);

    // Cream background, dark ink blob
    vec3 bgColor = vec3(0.94, 0.93, 0.91) + grain * 0.02;
    vec3 inkColor = vec3(0.06 + detail) + grain * 0.015;

    vec3 finalColor = mix(bgColor, inkColor, blob);

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
  const startTimeRef = useRef(Date.now())
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
        // Flush analyser buffer
        if (analyserRef.current) {
          const flush = new Uint8Array(analyserRef.current.frequencyBinCount)
          analyserRef.current.getByteFrequencyData(flush)
        }
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

  // Web Audio API: frequency analysis (frontend-only, zero latency)
  useEffect(() => {
    let active = true
    let audioCtx: AudioContext | null = null
    let analyser: AnalyserNode | null = null
    let dataArray: Uint8Array | null = null

    async function initWebAudio() {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
        if (!active) return

        audioCtx = new AudioContext()
        const source = audioCtx.createMediaStreamSource(stream)
        analyser = audioCtx.createAnalyser()
        analyser.fftSize = 2048
        analyser.smoothingTimeConstant = 0.6
        source.connect(analyser)
        analyserRef.current = analyser

        dataArray = new Uint8Array(analyser.frequencyBinCount)

        // Pump frequency data every frame, but only feed shader when recording
        function pump() {
          if (!active || !analyser || !dataArray) return
          analyser.getByteFrequencyData(dataArray as Uint8Array<ArrayBuffer>)

          if (stateRef.current === 1) {
            const bands = extractBands(dataArray, audioCtx!.sampleRate, analyser.fftSize)
            // Scale down and clamp to keep ink movement controlled
            amplitudeRef.current = Math.min(bands.rms * 0.5, 0.5)
            lowRef.current = Math.min(bands.low * 0.5, 0.5)
            midRef.current = Math.min(bands.mid * 0.5, 0.5)
            highRef.current = Math.min(bands.high * 0.5, 0.5)
          } else {
            // Decay to zero when not recording
            amplitudeRef.current *= 0.9
            lowRef.current *= 0.9
            midRef.current *= 0.9
            highRef.current *= 0.9
          }

          requestAnimationFrame(pump)
        }
        pump()
      } catch (err) {
        // Web Audio not available (e.g. no mic permission in webview)
        // Fall back to Tauri events (already set up above)
        console.warn("Web Audio unavailable, using Tauri amplitude events:", err)
      }
    }

    initWebAudio()

    return () => {
      active = false
      if (audioCtx) audioCtx.close()
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

    const resize = () => {
      const rect = canvas.parentElement!.getBoundingClientRect()
      const dpr = Math.min(window.devicePixelRatio || 1, 2)
      canvas.width = rect.width * dpr
      canvas.height = rect.height * dpr
      canvas.style.width = rect.width + "px"
      canvas.style.height = rect.height + "px"
      gl.viewport(0, 0, canvas.width, canvas.height)
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

    const lerpFactor = 0.08
    const stateLerpFactor = 0.05 // Slower spring for state transitions

    const render = () => {
      // Wrap time to prevent float precision issues at large values
      const elapsed = ((Date.now() - startTimeRef.current) / 1000) % 600

      // Smooth all values with hard clamp
      smoothAmpRef.current += (amplitudeRef.current - smoothAmpRef.current) * lerpFactor
      smoothLowRef.current += (lowRef.current - smoothLowRef.current) * lerpFactor
      smoothMidRef.current += (midRef.current - smoothMidRef.current) * lerpFactor
      smoothHighRef.current += (highRef.current - smoothHighRef.current) * lerpFactor
      smoothStateRef.current += (stateRef.current - smoothStateRef.current) * stateLerpFactor

      // Hard clamp smoothed values
      smoothAmpRef.current = Math.min(smoothAmpRef.current, 0.7)
      smoothLowRef.current = Math.min(smoothLowRef.current, 0.7)
      smoothMidRef.current = Math.min(smoothMidRef.current, 0.7)
      smoothHighRef.current = Math.min(smoothHighRef.current, 0.7)

      gl.useProgram(program)
      gl.uniform2f(resLoc, canvas.width, canvas.height)
      gl.uniform1f(timeLoc, elapsed)
      gl.uniform1f(ampLoc, smoothAmpRef.current)
      gl.uniform1f(lowLoc, smoothLowRef.current)
      gl.uniform1f(midLoc, smoothMidRef.current)
      gl.uniform1f(highLoc, smoothHighRef.current)
      gl.uniform1f(stateLoc, smoothStateRef.current)
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
