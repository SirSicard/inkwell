// The ink's motion: the design prototype's state machine and droplet physics, ported line for
// line. Doubles throughout, as in the prototype's JavaScript; values become Float only when packed
// into the uniform block, where the prototype's gl.uniform1f converts them. SimulationTests checks
// the port against the prototype's own numbers.
//
// One change of input, none of behaviour: the prototype drives the ink with a synthetic voice
// (`voice(t, seed)`, syllables near 4 Hz in phrases). The app drives it with the live audio's
// level (`InkVoice.levels`); the synthetic voice stays for the reference renders and the tests.
import Foundation

/// What the ink shows. Raw values are the prototype's `mode` names.
public enum InkState: String, CaseIterable, Sendable {
    /// Nothing live: one still drop.
    case idle
    /// A dictation: one wet drop that answers the voice.
    case dictating
    /// A meeting: your drop and the far end's, one per stream.
    case meeting
    /// The final pass: a blotting sheet lifts the ink.
    case blotting
    /// The far end is silent: its drop fades to a ghost with a seal-red outline.
    case problem

    /// Whether anything moves in this state (every state but idle).
    public var isLive: Bool { self != .idle }
}

/// Where the voice that moves the ink comes from.
public enum InkVoice: Sendable, Equatable {
    /// The prototype's stand-in voice. Reference renders and tests only.
    case synthetic
    /// Live levels, 0...1: the near end (your mic) and the far end.
    case levels(near: Double, far: Double)

    /// No voice at all.
    public static let silent = InkVoice.levels(near: 0, far: 0)
}

/// One droplet thrown off an ink body. Positions and radius are in p units (a pixel divided by
/// the canvas height).
public struct InkDroplet: Equatable, Sendable {
    public var alive = false
    public var x = 0.0, y = 0.0, vx = 0.0, vy = 0.0, r = 0.0
    /// 0 is your ink, 1 the far end's.
    public var ink = 0.0
    public var age = 0.0

    public init() {}

    init(alive: Bool, x: Double, y: Double, vx: Double, vy: Double, r: Double, ink: Double, age: Double) {
        self.alive = alive
        self.x = x
        self.y = y
        self.vx = vx
        self.vy = vy
        self.r = r
        self.ink = ink
        self.age = age
    }
}

/// The random draws a droplet's spawn takes (angle, speed, size): Math.random in the prototype.
public struct InkRandom: Sendable {
    private enum Source: Sendable {
        case system
        case mulberry32(UInt32)
    }

    private var source: Source

    /// The system's generator: the live app.
    public static let system = InkRandom(source: .system)

    /// mulberry32 from `seed`, as the prototype's check harness replaces Math.random with it.
    public static func seeded(_ seed: UInt32) -> InkRandom {
        InkRandom(source: .mulberry32(seed))
    }

    /// A value in 0..<1.
    public mutating func next() -> Double {
        switch source {
        case .system:
            return Double.random(in: 0..<1)
        case .mulberry32(var a):
            // The same 32-bit integer steps as the JavaScript (Math.imul is a wrapping multiply).
            a = a &+ 0x6D2B_79F5
            source = .mulberry32(a)
            var t = (a ^ (a >> 15)) &* (1 | a)
            t = (t &+ ((t ^ (t >> 7)) &* (61 | t))) ^ t
            return Double(t ^ (t >> 14)) / 4_294_967_296.0
        }
    }
}

/// The prototype's `_st`, `_drops` and `_step`. A value: the view that owns it steps it once per
/// frame on the main thread.
public struct InkSimulation: Sendable {
    // The prototype's props.
    public var state: InkState = .idle
    /// The pointer held down on the ink (the prototype's demo affordance): it wets the ink and
    /// makes it speak. The app has no such gesture; kept so the port stays whole.
    public var hold = false
    /// The prototype's `echo` prop: in a meeting, the near end copies the far end's synthetic
    /// voice. Synthetic voice only; real echo shows in the real levels.
    public var echo = false
    /// The body's centre height, 0...1 from the bottom (`cy`).
    public var cy = 0.5
    /// The canvas size in pixels (the prototype's `c.width`, `c.height`).
    public var canvasWidth = 360.0
    public var canvasHeight = 720.0

    // The prototype's `_st`.
    public internal(set) var t = 0.0
    public private(set) var wet = 0.0, two = 0.0, dead = 0.0, blot = 0.0, blotT = 0.0
    public private(set) var envA = 0.0, envB = 0.0, prevA = 0.0, prevB = 0.0
    public private(set) var coolA = 0.0, coolB = 0.0, breath = 0.0
    public private(set) var drops = [InkDroplet](repeating: InkDroplet(), count: 6)
    /// The Glow orb's and edge glow's weights: dictating, meeting, blotting and problem, each
    /// 0...1, eased toward the state's (`weights(for:)`) by 0.04 per 60 Hz frame on a time basis,
    /// so a state change fades in at any frame rate. A still frame has them at their targets.
    public private(set) var w = SIMD4<Double>(0, 0, 0, 0)
    /// Droplets spawned so far (diagnostics and tests).
    public private(set) var spawns = 0

    private var random: InkRandom

    /// `blotT` for the fixed blotting render: 1.38 s puts the 4.6 s cycle at 0.30, so blot = 0.5
    /// and the sheet sits mid-canvas.
    static let fixedBlotT = 1.38

    public init(random: InkRandom = .system) {
        self.random = random
    }

    /// The prototype's `_sm`: a smoothstep over -0.25...0.25.
    static func sm(_ x: Double) -> Double {
        let u = max(0, min(1, (x + 0.25) / 0.5))
        return u * u * (3 - 2 * u)
    }

    /// The prototype's stand-in voice: syllables near 4 Hz, grouped into phrases with pauses.
    static func voice(_ t: Double, _ seed: Double) -> Double {
        let syl = pow(max(0, sin(t * 2 * Double.pi * 4.1 + seed * 5)), 2.2)
        let phrase = sm(sin(t * 0.9 + seed * 2) + 0.6 * sin(t * 2.3 + seed) + 0.35)
        return syl * phrase * (0.62 + 0.38 * sin(t * 13.7 + seed * 9))
    }

    /// The prototype's `_step(dt)`. `snap` is its `_snap` (the `live = false` path): every spring
    /// and the envelope follower jump straight to their targets, and no droplet spawns.
    public mutating func step(_ dt: Double, snap: Bool, voice: InkVoice) {
        t += dt
        var tw = 0.0, t2 = 0.0, td = 0.0, vA = 0.0, vB = 0.0
        switch voice {
        case .synthetic:
            if state == .dictating {
                tw = 1
                vA = Self.voice(t, 0.3)
            }
            if state == .meeting {
                tw = 0.65
                t2 = 1
                let turn = sin(t * 0.42)
                vA = Self.voice(t, 0.3) * Self.sm(turn * 3 + 0.4)
                vB = Self.voice(t + 7.3, 2.1) * Self.sm(-turn * 3 + 0.4)
                if echo { vA = max(vA, vB * 0.85) }
            }
            if state == .problem {
                tw = 0.5
                t2 = 1
                td = 1
                vA = Self.voice(t, 0.3)
            }
        case .levels(let near, let far):
            // The same targets per state; the voice is the live audio instead.
            switch state {
            case .dictating:
                tw = 1
                vA = near
            case .meeting:
                tw = 0.65
                t2 = 1
                vA = near
                vB = far
            case .problem:
                tw = 0.5
                t2 = 1
                td = 1
                vA = near
            case .idle, .blotting:
                break
            }
        }
        if state == .blotting {
            t2 = 1
            blotT += dt
            let cyc = blotT.truncatingRemainder(dividingBy: 4.6) / 4.6  // JS `%` on positive doubles
            blot = min(1, cyc / 0.6)
            tw = cyc < 0.04 ? 0.9 : 0.9 * (1 - blot)
        } else {
            blot = 0
            blotT = 0
        }
        if hold {
            tw = 1
            vA = max(vA, 0.7 + 0.22 * sin(t * 9))
        }
        let k = snap ? 1 : 1 - exp(-dt * 3.2)
        wet += (tw - wet) * k
        two += (t2 - two) * k
        dead += (td - dead) * k
        // Envelope follower: fast attack, slow release, the way a level meter reads a voice.
        let atk = snap ? 1 : 1 - exp(-dt * 28), rel = snap ? 1 : 1 - exp(-dt * 6)
        envA += (vA - envA) * (vA > envA ? atk : rel)
        envB += (vB - envB) * (vB > envB ? atk : rel)
        breath = 0.5 + 0.5 * sin(t * 0.75)
        coolA -= dt
        coolB -= dt
        if !snap {
            // Each syllable onset throws a droplet that the body pulls back in.
            if envA > 0.5 && prevA <= 0.5 && coolA <= 0 {
                spawn(ink: 0)
                coolA = 0.22
            }
            if two > 0.5 && dead < 0.5 && envB > 0.5 && prevB <= 0.5 && coolB <= 0 {
                spawn(ink: 1)
                coolB = 0.26
            }
        }
        prevA = envA
        prevB = envB
        let kw = snap ? 1 : 1 - pow(1 - Self.weightEase, dt * 60)
        w += (Self.weights(for: state) - w) * kw
        physics(dt)
    }

    /// How far each weight moves toward its target per 60 Hz frame.
    static let weightEase = 0.04

    /// Each state's weights: dictating, meeting, blotting, problem. Blotting is a meeting being
    /// blotted, and a problem a meeting whose far end went quiet.
    public static func weights(for state: InkState) -> SIMD4<Double> {
        switch state {
        case .idle: SIMD4(0, 0, 0, 0)
        case .dictating: SIMD4(1, 0, 0, 0)
        case .meeting: SIMD4(0, 1, 0, 0)
        case .blotting: SIMD4(0, 1, 1, 0)
        case .problem: SIMD4(0, 1, 0, 1)
        }
    }

    /// The still frame: droplets cleared and every spring settled at its target, time unchanged.
    /// What idle shows, and what every state shows under Reduce Motion.
    public mutating func settle(voice: InkVoice) {
        clearDrops()
        step(0, snap: true, voice: voice)
    }

    /// The fixed state of a reference render at `fixedT`: a zeroed state, droplets off, then one
    /// `_step(0)` on the snap path. Blotting starts 1.38 s into its cycle. `voice` is the
    /// prototype's synthetic voice for a reference render, or fixed levels.
    public mutating func applyFixed(t fixedT: Double, voice: InkVoice = .synthetic) {
        t = fixedT
        wet = 0
        two = 0
        dead = 0
        blot = 0
        envA = 0
        envB = 0
        prevA = 0
        prevB = 0
        coolA = 0
        coolB = 0
        breath = 0
        w = .zero
        blotT = state == .blotting ? Self.fixedBlotT : 0
        hold = false
        echo = false
        clearDrops()
        step(0, snap: true, voice: voice)
    }

    private struct Geometry {
        var asp: Double, s: Double
        var a: (x: Double, y: Double), b: (x: Double, y: Double)
    }

    /// The prototype's `_geo`: the aspect, the scale and both ink centres, in p units.
    private func geometry() -> Geometry {
        let asp = canvasWidth / canvasHeight, s = min(asp, 1)
        let cx = 0.5 * asp
        return Geometry(asp: asp, s: s,
                        a: (cx - s * 0.10 * two, cy + s * 0.05 * two),
                        b: (cx + s * 0.19, cy - s * 0.19))
    }

    private mutating func spawn(ink: Int) {
        // A free slot, else the oldest. JS: reduce((a, b) => a.age > b.age ? a : b), so ties go
        // to the later droplet.
        var index = drops.firstIndex { !$0.alive } ?? 0
        if drops[index].alive {
            for i in 1..<drops.count where !(drops[index].age > drops[i].age) {
                index = i
            }
        }
        let g = geometry(), c = ink != 0 ? g.b : g.a
        let ang = random.next() * Double.pi * 2, sp = g.s * (0.5 + random.next() * 0.45)
        var d = drops[index]
        d.x = c.x + cos(ang) * g.s * 0.12
        d.y = c.y + sin(ang) * g.s * 0.12
        d.vx = cos(ang) * sp
        d.vy = sin(ang) * sp
        d.r = g.s * (ink != 0 ? 0.026 : 0.032) * (0.8 + random.next() * 0.5)
        d.ink = Double(ink)
        d.age = 0
        d.alive = true
        drops[index] = d
        spawns += 1
    }

    private mutating func physics(_ dt: Double) {
        let g = geometry()
        for i in drops.indices where drops[i].alive {
            var d = drops[i]
            d.age += dt
            let c = d.ink != 0 ? g.b : g.a, kx = c.x - d.x, ky = c.y - d.y
            d.vx += kx * 10 * dt
            d.vy += ky * 10 * dt
            let damp = exp(-dt * 3.4)
            d.vx *= damp
            d.vy *= damp
            d.x += d.vx * dt
            d.y += d.vy * dt
            // Droplets stay on the sheet: the old blob ran off the panel edge and got cut straight.
            let lo = g.s * 0.1, hiX = g.asp - g.s * 0.1
            if d.x < lo || d.x > hiX {
                d.vx *= -0.4
                d.x = min(hiX, max(lo, d.x))
            }
            if d.y < 0.06 || d.y > 0.94 {
                d.vy *= -0.4
                d.y = min(0.94, max(0.06, d.y))
            }
            if d.age > 0.55 && hypot(kx, ky) < g.s * 0.1 { d.r *= exp(-dt * 5) }
            if d.age > 2.6 { d.r *= exp(-dt * 4) }
            if d.r < g.s * 0.004 { d.alive = false }
            drops[i] = d
        }
    }

    private mutating func clearDrops() {
        for i in drops.indices {
            drops[i] = InkDroplet()
        }
    }

    /// The orb's uniform block for this state (shaders/ink.wgsl, `G`): the canvas, where the orb
    /// sits and how large a unit is, the time, your level and theirs (the envelopes), the state
    /// weights, and the theme's colours. `motion` false draws a still frame: the shader stops its
    /// time.
    public func uniforms(palette: OrbPalette, placement: OrbPlacement, motion: Bool) -> InkUniforms {
        var u = InkUniforms()
        u.res = SIMD2(Float(canvasWidth), Float(canvasHeight))
        u.center = SIMD2(Float(canvasWidth * placement.x), Float(canvasHeight * placement.yFromTop))
        u.time = Float(t)
        u.unit = Float(min(canvasWidth, canvasHeight) * placement.unit)
        u.you = Float(envA)
        u.them = Float(envB)
        u.w = SIMD4<Float>(w)
        u.dark = palette.dark ? 1 : 0
        u.motion = motion ? 1 : 0
        u.yA = SIMD4(palette.yA, 0)
        u.yB = SIMD4(palette.yB, 0)
        u.tA = SIMD4(palette.tA, 0)
        u.tB = SIMD4(palette.tB, 0)
        u.idle = SIMD4(palette.idle, 0)
        u.ink = SIMD4(palette.ink, 0)
        return u
    }
}

/// The shader's uniform block `G` (shaders/ink.wgsl), 160 bytes: the canvas and the orb's centre
/// in pixels (top-left origin), the time, the unit, your level and theirs, the four state weights,
/// dark and motion, then six colours (rgb, the fourth unused). A plain value, so a frame hands it
/// to Metal without allocating; Swift lays it out at the shader's offsets (SimulationTests checks).
public struct InkUniforms: Equatable, Sendable {
    public var res = SIMD2<Float>(0, 0)
    public var center = SIMD2<Float>(0, 0)
    public var time: Float = 0
    public var unit: Float = 0
    public var you: Float = 0
    public var them: Float = 0
    /// Dictating, meeting, blotting, problem.
    public var w = SIMD4<Float>(0, 0, 0, 0)
    public var dark: Float = 0
    public var motion: Float = 0
    public var pad = SIMD2<Float>(0, 0)
    public var yA = SIMD4<Float>(0, 0, 0, 0)
    public var yB = SIMD4<Float>(0, 0, 0, 0)
    public var tA = SIMD4<Float>(0, 0, 0, 0)
    public var tB = SIMD4<Float>(0, 0, 0, 0)
    public var idle = SIMD4<Float>(0, 0, 0, 0)
    public var ink = SIMD4<Float>(0, 0, 0, 0)

    public init() {}
}
