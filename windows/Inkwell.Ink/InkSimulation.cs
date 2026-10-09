// The ink's motion: the design prototype's state machine and droplet physics, ported line for line
// from the Mac renderer (mac/Sources/InkRenderer/InkSimulation.swift, itself a port of the
// prototype's JavaScript). Doubles throughout; values become float only when packed into the
// uniform block. InkSimulationTests checks it against the prototype's own numbers, as the Mac's
// SimulationTests do. Glow keeps it for the state and the levels (the envelope follower): the orb's
// uniform block is the state weights, smoothed here, and the envelopes as your level and theirs.
using System.Numerics;
using System.Runtime.InteropServices;

namespace Inkwell.Ink;

/// <summary>One droplet thrown off an ink body, in p units (a pixel divided by the canvas height).</summary>
public record struct InkDroplet(bool Alive, double X, double Y, double Vx, double Vy, double R, double Ink, double Age);

/// <summary>The random draws a droplet's spawn takes: Math.random in the prototype.</summary>
public struct InkRandom
{
    private readonly bool seeded;
    private uint state;

    private InkRandom(bool seeded, uint state)
    {
        this.seeded = seeded;
        this.state = state;
    }

    /// <summary>The system's generator: the live app.</summary>
    public static InkRandom System => new(false, 0);

    /// <summary>mulberry32 from <paramref name="seed"/>, as the prototype's check harness uses.</summary>
    public static InkRandom Seeded(uint seed) => new(true, seed);

    /// <summary>A value in [0, 1).</summary>
    public double Next()
    {
        if (!seeded)
        {
            return Random.Shared.NextDouble();
        }
        // The same 32-bit integer steps as the JavaScript (Math.imul is a wrapping multiply).
        unchecked
        {
            state += 0x6D2B79F5;
            var a = state;
            var t = (a ^ (a >> 15)) * (1 | a);
            t = (t + ((t ^ (t >> 7)) * (61 | t))) ^ t;
            return (t ^ (t >> 14)) / 4_294_967_296.0;
        }
    }
}

/// <summary>The prototype's <c>_st</c>, <c>_drops</c> and <c>_step</c>. The surface that owns it steps it once per frame on the UI thread.</summary>
public sealed class InkSimulation(InkRandom random)
{
    /// <summary>A simulation drawing on the system's random numbers.</summary>
    public InkSimulation()
        : this(InkRandom.System)
    {
    }

    private InkRandom random = random;

    // The prototype's props.
    /// <summary>What the ink shows.</summary>
    public InkState State { get; set; } = InkState.Idle;
    /// <summary>The pointer held down on the ink (the prototype's demo affordance). Kept so the port stays whole.</summary>
    public bool Hold { get; set; }
    /// <summary>The prototype's <c>echo</c> prop (synthetic voice only).</summary>
    public bool Echo { get; set; }
    /// <summary>The body's centre height, 0..1 from the bottom (<c>cy</c>).</summary>
    public double Cy { get; set; } = 0.5;
    /// <summary>The canvas width in pixels.</summary>
    public double CanvasWidth { get; set; } = 360;
    /// <summary>The canvas height in pixels.</summary>
    public double CanvasHeight { get; set; } = 720;

    // The prototype's _st.
    /// <summary>Seconds.</summary>
    public double T { get; set; }
    /// <summary>The wet spring.</summary>
    public double Wet { get; private set; }
    /// <summary>The second-ink spring.</summary>
    public double Two { get; private set; }
    /// <summary>The far-end-silent spring.</summary>
    public double Dead { get; private set; }
    /// <summary>The blotting sheet's position.</summary>
    public double Blot { get; private set; }
    /// <summary>Time into the blotting cycle.</summary>
    public double BlotT { get; private set; }
    /// <summary>The near end's envelope.</summary>
    public double EnvA { get; private set; }
    /// <summary>The far end's envelope.</summary>
    public double EnvB { get; private set; }
    /// <summary>Droplet cool-down, near end.</summary>
    public double CoolA { get; private set; }
    /// <summary>Droplet cool-down, far end.</summary>
    public double CoolB { get; private set; }
    /// <summary>The slow breath.</summary>
    public double Breath { get; private set; }
    private double prevA, prevB;
    private readonly InkDroplet[] drops = new InkDroplet[6];

    /// <summary>The six droplets.</summary>
    public ReadOnlySpan<InkDroplet> Drops => drops;
    /// <summary>Droplets spawned so far (diagnostics and tests).</summary>
    public int Spawns { get; private set; }

    /// <summary>
    /// Glow's state weights, each 0..1: dictating, meeting, blotting, problem. Each moves towards
    /// its state's target by 4 % per 60 Hz frame, on a time basis; the snap path sets them.
    /// </summary>
    public (double Dictating, double Meeting, double Blotting, double Problem) Weights { get; private set; }

    /// <summary>
    /// How far the final pass's blot goes, 0..1: 1 (the Drop) blots down to one small drop of ink;
    /// the window's orb stops at 0.45, where the orbs have merged, shrunk and taken the ink colour
    /// with a soft edge, so a wide panel behind the text never ends on a hard dot.
    /// </summary>
    public double BlotDepth { get; set; } = 1;

    /// <summary>The weights a state settles at, the blot going <paramref name="blotDepth"/> of the way.</summary>
    public static (double Dictating, double Meeting, double Blotting, double Problem) WeightsFor(InkState state, double blotDepth = 1) => state switch
    {
        InkState.Dictating => (1, 0, 0, 0),
        InkState.Meeting => (0, 1, 0, 0),
        InkState.Blotting => (0, 1, blotDepth, 0),
        // Still a meeting, whose far end has gone silent.
        InkState.Problem => (0, 1, 0, 1),
        _ => (0, 0, 0, 0),
    };

    /// <summary>What a frame shows: the weights, the envelopes as your level and theirs, and the time.</summary>
    public GlowFrame Frame(bool moving) =>
        new(T, Weights.Dictating, Weights.Meeting, Weights.Blotting, Weights.Problem, EnvA, EnvB, moving);

    /// <summary><c>blotT</c> for the fixed blotting render: 1.38 s puts the 4.6 s cycle at 0.30, so blot = 0.5.</summary>
    internal const double FixedBlotT = 1.38;

    /// <summary>The prototype's <c>_sm</c>: a smoothstep over -0.25..0.25.</summary>
    internal static double Sm(double x)
    {
        var u = Math.Max(0, Math.Min(1, (x + 0.25) / 0.5));
        return u * u * (3 - 2 * u);
    }

    /// <summary>The prototype's stand-in voice: syllables near 4 Hz, grouped into phrases with pauses.</summary>
    internal static double Voice(double t, double seed)
    {
        var syl = Math.Pow(Math.Max(0, Math.Sin(t * 2 * Math.PI * 4.1 + seed * 5)), 2.2);
        var phrase = Sm(Math.Sin(t * 0.9 + seed * 2) + 0.6 * Math.Sin(t * 2.3 + seed) + 0.35);
        return syl * phrase * (0.62 + 0.38 * Math.Sin(t * 13.7 + seed * 9));
    }

    /// <summary>
    /// The prototype's <c>_step(dt)</c>. <paramref name="snap"/> is its <c>_snap</c> path: every
    /// spring and the envelope follower jump to their targets, and no droplet spawns.
    /// </summary>
    public void Step(double dt, bool snap, InkVoice voice)
    {
        T += dt;
        double tw = 0, t2 = 0, td = 0, vA = 0, vB = 0;
        if (voice.IsSynthetic)
        {
            if (State == InkState.Dictating)
            {
                tw = 1;
                vA = Voice(T, 0.3);
            }
            if (State == InkState.Meeting)
            {
                tw = 0.65;
                t2 = 1;
                var turn = Math.Sin(T * 0.42);
                vA = Voice(T, 0.3) * Sm(turn * 3 + 0.4);
                vB = Voice(T + 7.3, 2.1) * Sm(-turn * 3 + 0.4);
                if (Echo)
                {
                    vA = Math.Max(vA, vB * 0.85);
                }
            }
            if (State == InkState.Problem)
            {
                tw = 0.5;
                t2 = 1;
                td = 1;
                vA = Voice(T, 0.3);
            }
        }
        else
        {
            // The same targets per state; the voice is the live audio instead.
            switch (State)
            {
                case InkState.Dictating:
                    tw = 1;
                    vA = voice.Near;
                    break;
                case InkState.Meeting:
                    tw = 0.65;
                    t2 = 1;
                    vA = voice.Near;
                    vB = voice.Far;
                    break;
                case InkState.Problem:
                    tw = 0.5;
                    t2 = 1;
                    td = 1;
                    vA = voice.Near;
                    break;
                case InkState.Idle:
                case InkState.Blotting:
                default:
                    break;
            }
        }
        if (State == InkState.Blotting)
        {
            t2 = 1;
            BlotT += dt;
            var cyc = BlotT % 4.6 / 4.6; // JS `%` on positive doubles
            Blot = Math.Min(1, cyc / 0.6);
            tw = cyc < 0.04 ? 0.9 : 0.9 * (1 - Blot);
        }
        else
        {
            Blot = 0;
            BlotT = 0;
        }
        if (Hold)
        {
            tw = 1;
            vA = Math.Max(vA, 0.7 + 0.22 * Math.Sin(T * 9));
        }
        var k = snap ? 1 : 1 - Math.Exp(-dt * 3.2);
        // Glow's weights: 0.04 per 60 Hz frame, whatever the frame rate.
        var kw = snap ? 1 : 1 - Math.Pow(1 - 0.04, dt * 60);
        var target = WeightsFor(State, BlotDepth);
        Weights = (
            Weights.Dictating + (target.Dictating - Weights.Dictating) * kw,
            Weights.Meeting + (target.Meeting - Weights.Meeting) * kw,
            Weights.Blotting + (target.Blotting - Weights.Blotting) * kw,
            Weights.Problem + (target.Problem - Weights.Problem) * kw);
        Wet += (tw - Wet) * k;
        Two += (t2 - Two) * k;
        Dead += (td - Dead) * k;
        // Envelope follower: fast attack, slow release, the way a level meter reads a voice.
        var atk = snap ? 1 : 1 - Math.Exp(-dt * 28);
        var rel = snap ? 1 : 1 - Math.Exp(-dt * 6);
        EnvA += (vA - EnvA) * (vA > EnvA ? atk : rel);
        EnvB += (vB - EnvB) * (vB > EnvB ? atk : rel);
        Breath = 0.5 + 0.5 * Math.Sin(T * 0.75);
        CoolA -= dt;
        CoolB -= dt;
        if (!snap)
        {
            // Each syllable onset throws a droplet that the body pulls back in.
            if (EnvA > 0.5 && prevA <= 0.5 && CoolA <= 0)
            {
                Spawn(0);
                CoolA = 0.22;
            }
            if (Two > 0.5 && Dead < 0.5 && EnvB > 0.5 && prevB <= 0.5 && CoolB <= 0)
            {
                Spawn(1);
                CoolB = 0.26;
            }
        }
        prevA = EnvA;
        prevB = EnvB;
        Physics(dt);
    }

    /// <summary>The still frame: droplets cleared and every spring settled at its target, time unchanged.</summary>
    public void Settle(InkVoice voice)
    {
        ClearDrops();
        Step(0, snap: true, voice);
    }

    /// <summary>
    /// The fixed state of a reference render at <paramref name="fixedT"/>: a zeroed state, droplets
    /// off, then one <c>_step(0)</c> on the snap path. Blotting starts 1.38 s into its cycle.
    /// </summary>
    public void ApplyFixed(double fixedT, InkVoice voice)
    {
        T = fixedT;
        Wet = Two = Dead = Blot = EnvA = EnvB = CoolA = CoolB = Breath = 0;
        Weights = default;
        prevA = prevB = 0;
        BlotT = State == InkState.Blotting ? FixedBlotT : 0;
        Hold = false;
        Echo = false;
        ClearDrops();
        Step(0, snap: true, voice);
    }

    private readonly record struct Geometry(double Asp, double S, double Ax, double Ay, double Bx, double By);

    /// <summary>The prototype's <c>_geo</c>: the aspect, the scale and both ink centres, in p units.</summary>
    private Geometry Geo()
    {
        var asp = CanvasWidth / CanvasHeight;
        var s = Math.Min(asp, 1);
        var cx = 0.5 * asp;
        return new Geometry(asp, s, cx - s * 0.10 * Two, Cy + s * 0.05 * Two, cx + s * 0.19, Cy - s * 0.19);
    }

    private void Spawn(int ink)
    {
        // A free slot, else the oldest. JS: reduce((a, b) => a.age > b.age ? a : b), so ties go
        // to the later droplet.
        var index = Array.FindIndex(drops, d => !d.Alive);
        if (index < 0)
        {
            index = 0;
            for (var i = 1; i < drops.Length; i++)
            {
                if (!(drops[index].Age > drops[i].Age))
                {
                    index = i;
                }
            }
        }
        var g = Geo();
        var (cx, cy) = ink != 0 ? (g.Bx, g.By) : (g.Ax, g.Ay);
        var ang = random.Next() * Math.PI * 2;
        var sp = g.S * (0.5 + random.Next() * 0.45);
        var r = g.S * (ink != 0 ? 0.026 : 0.032) * (0.8 + random.Next() * 0.5);
        drops[index] = new InkDroplet(
            Alive: true,
            X: cx + Math.Cos(ang) * g.S * 0.12,
            Y: cy + Math.Sin(ang) * g.S * 0.12,
            Vx: Math.Cos(ang) * sp,
            Vy: Math.Sin(ang) * sp,
            R: r,
            Ink: ink,
            Age: 0);
        Spawns++;
    }

    private void Physics(double dt)
    {
        var g = Geo();
        for (var i = 0; i < drops.Length; i++)
        {
            var d = drops[i];
            if (!d.Alive)
            {
                continue;
            }
            d.Age += dt;
            var (cx, cy) = d.Ink != 0 ? (g.Bx, g.By) : (g.Ax, g.Ay);
            double kx = cx - d.X, ky = cy - d.Y;
            d.Vx += kx * 10 * dt;
            d.Vy += ky * 10 * dt;
            var damp = Math.Exp(-dt * 3.4);
            d.Vx *= damp;
            d.Vy *= damp;
            d.X += d.Vx * dt;
            d.Y += d.Vy * dt;
            // Droplets stay on the sheet: the old blob ran off the panel edge and got cut straight.
            double lo = g.S * 0.1, hiX = g.Asp - g.S * 0.1;
            if (d.X < lo || d.X > hiX)
            {
                d.Vx *= -0.4;
                d.X = Math.Min(hiX, Math.Max(lo, d.X));
            }
            if (d.Y < 0.06 || d.Y > 0.94)
            {
                d.Vy *= -0.4;
                d.Y = Math.Min(0.94, Math.Max(0.06, d.Y));
            }
            if (d.Age > 0.55 && double.Hypot(kx, ky) < g.S * 0.1)
            {
                d.R *= Math.Exp(-dt * 5);
            }
            if (d.Age > 2.6)
            {
                d.R *= Math.Exp(-dt * 4);
            }
            if (d.R < g.S * 0.004)
            {
                d.Alive = false;
            }
            drops[i] = d;
        }
    }

    private void ClearDrops() => Array.Clear(drops);

    /// <summary>
    /// The orb's uniform block for this frame (C4): the canvas, where the orb sits, the time, your
    /// level and theirs (the envelopes), the state weights, the mode, and the colours, with the rest
    /// tint in idle's fourth lane and rest boost in ink's (the uniform block keeps its size).
    /// <paramref name="moving"/> false is the still frame: the shader holds its time at 0.
    /// </summary>
    public InkUniforms Uniforms(InkPlacement placement, GlowLook look, bool moving)
    {
        ArgumentNullException.ThrowIfNull(look);
        var shorter = Math.Min(CanvasWidth, CanvasHeight);
        return new InkUniforms
        {
            ResX = (float)CanvasWidth,
            ResY = (float)CanvasHeight,
            CenterX = (float)(CanvasWidth * placement.X),
            CenterY = (float)(CanvasHeight * placement.Y),
            Time = (float)T,
            Unit = (float)(shorter * placement.Unit),
            You = (float)Math.Clamp(EnvA, 0, 1),
            Them = (float)Math.Clamp(EnvB, 0, 1),
            Dictating = (float)Weights.Dictating,
            Meeting = (float)Weights.Meeting,
            Blotting = (float)Weights.Blotting,
            Problem = (float)Weights.Problem,
            Dark = look.Dark ? 1 : 0,
            Motion = moving ? 1 : 0,
            YouA = Vec(look.YouA),
            YouB = Vec(look.YouB),
            ThemA = Vec(look.ThemA),
            ThemB = Vec(look.ThemB),
            Idle = new Vector4(look.Idle.R, look.Idle.G, look.Idle.B, Math.Clamp(look.RestTint, 0, 1)),
            Ink = new Vector4(look.Ink.R, look.Ink.G, look.Ink.B, Math.Clamp(look.RestBoost, 0, 1)),
        };
    }

    private static Vector4 Vec((float R, float G, float B) c) => new(c.R, c.G, c.B, 0);
}

/// <summary>
/// One frame of Glow: the time, the state weights, your level and theirs (0..1), and whether it
/// moves (false: the still frame, whose time stands still).
/// </summary>
public readonly record struct GlowFrame(double Time, double Dictating, double Meeting, double Blotting, double Problem, double You, double Them, bool Moving);

/// <summary>
/// The shader's uniform block <c>G</c> (shaders/ink.wgsl), 160 bytes: the canvas and the orb's
/// centre (pixels, top-left origin), the time, the unit, your level and theirs, the four state
/// weights, the mode and whether it moves, then six colours as vec4 (idle.a tint, ink.a boost) from offset
/// 64. The HLSL cbuffer packs the same way.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public struct InkUniforms
{
    /// <summary>Canvas width in pixels.</summary>
    public float ResX;
    /// <summary>Canvas height in pixels.</summary>
    public float ResY;
    /// <summary>The orb's centre, pixels from the left.</summary>
    public float CenterX;
    /// <summary>The orb's centre, pixels from the top.</summary>
    public float CenterY;
    /// <summary>Seconds.</summary>
    public float Time;
    /// <summary>Pixels per orb unit.</summary>
    public float Unit;
    /// <summary>Your level, 0..1.</summary>
    public float You;
    /// <summary>The far end's level, 0..1.</summary>
    public float Them;
    /// <summary>The state weights (w), each 0..1.</summary>
    public float Dictating;
    public float Meeting;
    public float Blotting;
    public float Problem;
    /// <summary>1 at night, 0 by day.</summary>
    public float Dark;
    /// <summary>1 animates; 0 is the still frame.</summary>
    public float Motion;
    /// <summary>Unused (pads the colours to offset 64).</summary>
    public float Pad0;
    public float Pad1;
    /// <summary>Your colour and its partner shade.</summary>
    public Vector4 YouA;
    public Vector4 YouB;
    /// <summary>The far end's colour and its partner shade.</summary>
    public Vector4 ThemA;
    public Vector4 ThemB;
    /// <summary>The orb at rest.</summary>
    public Vector4 Idle;
    /// <summary>The drop it blots down to.</summary>
    public Vector4 Ink;
}
