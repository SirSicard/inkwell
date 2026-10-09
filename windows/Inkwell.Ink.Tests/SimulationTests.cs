// The ink's state machine and droplet physics against the design prototype's own JavaScript: the
// Mac's SimulationTests (mac/Tests/InkRendererTests), same numbers, same tolerance. The prototype
// ran with Math.random replaced by mulberry32, as InkRandom.Seeded does.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class SimulationTests
{
    private const double Tolerance = 1e-9;

    [Fact]
    public void TheFixedStateOfEveryStateAtT12()
    {
        (InkState State, double Wet, double Two, double Dead, double Blot)[] expected =
        [
            (InkState.Idle, 0, 0, 0, 0),
            (InkState.Dictating, 1, 0, 0, 0),
            (InkState.Meeting, 0.65, 1, 0, 0),
            (InkState.Blotting, 0.45, 1, 0, 0.5),
            (InkState.Problem, 0.5, 1, 1, 0),
        ];
        foreach (var (state, wet, two, dead, blot) in expected)
        {
            var sim = new InkSimulation { State = state };
            sim.ApplyFixed(12, InkVoice.Synthetic);
            Assert.Equal(12, sim.T);
            Assert.Equal(wet, sim.Wet, Tolerance);
            Assert.Equal(two, sim.Two, Tolerance);
            Assert.Equal(dead, sim.Dead, Tolerance);
            Assert.Equal(blot, sim.Blot, Tolerance);
            Assert.Equal(0, sim.EnvA);
            Assert.Equal(0, sim.EnvB);
            Assert.Equal(0.706059, sim.Breath, 1e-6);
            Assert.DoesNotContain(sim.Drops.ToArray(), d => d.Alive);
        }
    }

    [Fact]
    public void TheFixedStateAtT1512CarriesTheVoice()
    {
        (InkState State, double EnvA)[] expected =
        [
            (InkState.Idle, 0), (InkState.Dictating, 0.834168), (InkState.Meeting, 0.834168),
            (InkState.Blotting, 0), (InkState.Problem, 0.834168),
        ];
        foreach (var (state, envA) in expected)
        {
            var sim = new InkSimulation { State = state };
            sim.ApplyFixed(15.12, InkVoice.Synthetic);
            Assert.Equal(envA, sim.EnvA, 1e-6);
            Assert.Equal(0, sim.EnvB, 1e-12);
            Assert.Equal(0.029365, sim.Breath, 1e-6);
        }
    }

    [Fact]
    public void DictatingFor600LiveStepsMatchesThePrototype()
    {
        var sim = Run(InkState.Dictating, 600);
        ExpectState(sim, t: 21.999999999999858, wet: 0.9999999999999875, two: 0, dead: 0,
            envA: 0.6144269784439061, envB: 0, coolA: 0.17, coolB: -10.000000000000076, breath: 0.1441073288154759);
        Assert.Equal(12, sim.Spawns);
        ExpectDrops(sim,
        [
            new(true, 0.253184919774139, 0.4400177995046888, -0.00754496737096397, 0.14209580701230884, 0.01856758185401559, 0, 0.55),
            new(true, 0.17052380212789878, 0.5235585719183927, -0.2848474047353476, 0.08443531837071983, 0.02075627201162279, 0, 0.06666666666666667),
            new(true, 0.2511811072647956, 0.5041106485861104, -0.02671918248261253, -0.0929916976788635, 0.003242740798292024, 0, 1.0333333333333345),
            default, default, default,
        ]);
    }

    [Fact]
    public void AMeetingFor900LiveStepsMatchesThePrototype()
    {
        var sim = Run(InkState.Meeting, 900);
        ExpectState(sim, t: 26.999999999999574, wet: 0.649999999999999, two: 0.999999999999999, dead: 0,
            envA: 1.870380836993794e-12, envB: 0.007143259207834781, coolA: -4.346666666666657,
            coolB: -0.4733333333333332, breath: 0.9927625557825327);
        Assert.Equal(18, sim.Spawns);
        ExpectDrops(sim,
        [
            new(false, 0.34918316573329206, 0.409301029980147, -0.06352888333281757, -0.0653188635690675, 0.0018622836576670818, 1, 0.9833333333333347),
            new(false, 0.34550002695732973, 0.405482857176386, 0.06213916870072609, 0.06000545191008686, 0.00191059691410236, 1, 1.100000000000001),
            new(true, 0.37462224794216203, 0.4021434750802657, -0.15090867964798024, 0.01115891139625417, 0.007262791883202992, 1, 0.7500000000000007),
            new(false, 0.349491018698219, 0.40366875882564107, 0.06585327922175835, -0.019520425689014926, 0.0019319743033900463, 1, 1.1500000000000008),
            default, default,
        ]);
    }

    [Fact]
    public void BlottingCyclesTheSheetAndThrowsNothing()
    {
        var sim = Run(InkState.Blotting, 600);
        Assert.Equal(0.28985507246379605, sim.Blot, Tolerance);
        Assert.Equal(0.6669197833902668, sim.Wet, Tolerance);
        Assert.Equal(0, sim.Spawns);
    }

    [Fact]
    public void DropletsStayOnTheSheet()
    {
        foreach (var (w, h) in new[] { (360.0, 720.0), (84.0, 84.0), (112.0, 1400.0), (900.0, 300.0) })
        {
            var sim = new InkSimulation(InkRandom.Seeded(3)) { CanvasWidth = w, CanvasHeight = h, State = InkState.Meeting };
            double asp = w / h, s = Math.Min(asp, 1);
            for (var step = 0; step < 3000; step++)
            {
                sim.Step(1.0 / 60, snap: false, InkVoice.Levels(1, step % 40 < 20 ? 1 : 0));
                foreach (var d in sim.Drops)
                {
                    if (!d.Alive)
                    {
                        continue;
                    }
                    Assert.InRange(d.X, s * 0.1 - 1e-12, asp - s * 0.1 + 1e-12);
                    Assert.InRange(d.Y, 0.06 - 1e-12, 0.94 + 1e-12);
                }
            }
            Assert.True(sim.Spawns > 10, $"the loud meeting threw droplets at {w}x{h}");
        }
    }

    [Fact]
    public void LevelsDriveTheEnvelopesOnlyWhereTheStateListens()
    {
        (InkState State, bool A, bool B)[] cases =
        [
            (InkState.Idle, false, false), (InkState.Dictating, true, false), (InkState.Meeting, true, true),
            (InkState.Blotting, false, false), (InkState.Problem, true, false),
        ];
        foreach (var (state, a, b) in cases)
        {
            var sim = new InkSimulation(InkRandom.Seeded(1)) { State = state };
            sim.Step(0, snap: true, InkVoice.Levels(0.8, 0.6));
            Assert.Equal(a ? 0.8 : 0, sim.EnvA, 1e-12);
            Assert.Equal(b ? 0.6 : 0, sim.EnvB, 1e-12);
        }
    }

    [Fact]
    public void SettlingForAStillFrameClearsDropletsAndSnapsTheSprings()
    {
        var sim = Run(InkState.Dictating, 600);
        Assert.Contains(sim.Drops.ToArray(), d => d.Alive);
        sim.State = InkState.Idle;
        var t = sim.T;
        sim.Settle(InkVoice.Silent);
        Assert.DoesNotContain(sim.Drops.ToArray(), d => d.Alive);
        Assert.Equal(0, sim.Wet);
        Assert.Equal(0, sim.EnvA);
        Assert.Equal(t, sim.T);
    }

    [Fact]
    public unsafe void TheUniformBlockMatchesTheShader()
    {
        // G in shaders/ink.wgsl: 160 bytes, the colours from offset 64.
        Assert.Equal(160, sizeof(InkUniforms));
        var u0 = default(InkUniforms);
        var at = (byte*)&u0;
        Assert.Equal(8, (int)((byte*)&u0.CenterX - at));
        Assert.Equal(16, (int)((byte*)&u0.Time - at));
        Assert.Equal(32, (int)((byte*)&u0.Dictating - at));
        Assert.Equal(48, (int)((byte*)&u0.Dark - at));
        Assert.Equal(52, (int)((byte*)&u0.Motion - at));
        Assert.Equal(64, (int)((byte*)&u0.YouA - at));
        Assert.Equal(144, (int)((byte*)&u0.Ink - at));

        var sim = Run(InkState.Meeting, 900);
        var u = sim.Uniforms(new InkPlacement(0.56, 0.26, 0.72), GlowLook.Default, moving: true);
        Assert.Equal(360f, u.ResX);
        Assert.Equal(720f, u.ResY);
        Assert.Equal((float)(360 * 0.56), u.CenterX);
        Assert.Equal((float)(720 * 0.26), u.CenterY);
        Assert.Equal((float)(360 * 0.72), u.Unit); // the shorter side
        Assert.Equal((float)sim.T, u.Time);
        Assert.Equal((float)sim.EnvB, u.Them);
        Assert.Equal(1f, u.Motion);
        Assert.Equal(0f, u.Dark);
        Assert.Equal(GlowLook.Default.YouA.R, u.YouA.X);
        // After 900 frames of a meeting the weights have settled at the meeting's.
        Assert.Equal(1f, u.Meeting, 3);
        Assert.Equal(0f, u.Dictating, 3);
        Assert.Equal(0f, sim.Uniforms(InkPlacement.Centre, GlowLook.Default, moving: false).Motion);
    }

    [Fact]
    public void TheWeightsFollowTheStateAtFourPercentPerFrame()
    {
        var sim = new InkSimulation(InkRandom.Seeded(1)) { State = InkState.Dictating };
        sim.Step(1 / 60.0, snap: false, InkVoice.Silent);
        Assert.Equal(0.04, sim.Weights.Dictating, 9);
        // Twice as long a frame moves it as two frames would.
        var slow = new InkSimulation(InkRandom.Seeded(1)) { State = InkState.Dictating };
        slow.Step(2 / 60.0, snap: false, InkVoice.Silent);
        Assert.Equal(1 - 0.96 * 0.96, slow.Weights.Dictating, 9);
        sim.State = InkState.Blotting;
        sim.Settle(InkVoice.Silent);
        Assert.Equal((0.0, 1.0, 1.0, 0.0), sim.Weights);
        Assert.Equal((0.0, 1.0, 0.0, 1.0), InkSimulation.WeightsFor(InkState.Problem));
    }

    /// <summary>
    /// Soft blotting: with a blot depth below 1 the final pass's blot stops partway (the window's
    /// orb, 0.45: the orbs merge, shrink and take the ink colour, with a soft edge); the Drop keeps
    /// the full blot (depth 1, the default).
    /// </summary>
    [Fact]
    public void ABlotDepthStopsTheBlotPartway()
    {
        Assert.Equal((0.0, 1.0, 0.45, 0.0), InkSimulation.WeightsFor(InkState.Blotting, 0.45));
        var soft = new InkSimulation(InkRandom.Seeded(1)) { State = InkState.Blotting, BlotDepth = 0.45 };
        soft.Settle(InkVoice.Silent);
        Assert.Equal(0.45f, soft.Uniforms(InkPlacement.Centre, GlowLook.Default, moving: false).Blotting, 6);
        for (var i = 0; i < 600; i++)
        {
            soft.Step(1 / 60.0, snap: false, InkVoice.Silent);
        }
        Assert.Equal(0.45, soft.Weights.Blotting, 6);
        var full = new InkSimulation(InkRandom.Seeded(1)) { State = InkState.Blotting };
        full.Settle(InkVoice.Silent);
        Assert.Equal(1f, full.Uniforms(InkPlacement.Centre, GlowLook.Default, moving: false).Blotting, 6);
    }

    [Fact]
    public void Mulberry32MatchesItsReferenceSequence()
    {
        var rng = InkRandom.Seeded(1);
        Assert.Equal(0.6270739405881613, rng.Next(), 1e-15);
        Assert.Equal(0.002735721180215478, rng.Next(), 1e-15);
        Assert.Equal(0.5274470399599522, rng.Next(), 1e-15);
    }

    private static InkSimulation Run(InkState state, int steps)
    {
        var sim = new InkSimulation(InkRandom.Seeded(1)) { State = state, T = 12 };
        for (var i = 0; i < steps; i++)
        {
            sim.Step(1.0 / 60.0, snap: false, InkVoice.Synthetic);
        }
        return sim;
    }

    private static void ExpectState(InkSimulation sim, double t, double wet, double two, double dead, double envA,
        double envB, double coolA, double coolB, double breath)
    {
        Assert.Equal(t, sim.T, Tolerance);
        Assert.Equal(wet, sim.Wet, Tolerance);
        Assert.Equal(two, sim.Two, Tolerance);
        Assert.Equal(dead, sim.Dead, Tolerance);
        Assert.Equal(envA, sim.EnvA, Tolerance);
        Assert.Equal(envB, sim.EnvB, Tolerance);
        Assert.Equal(coolA, sim.CoolA, Tolerance);
        Assert.Equal(coolB, sim.CoolB, Tolerance);
        Assert.Equal(breath, sim.Breath, Tolerance);
    }

    private static void ExpectDrops(InkSimulation sim, InkDroplet[] expected)
    {
        Assert.Equal(expected.Length, sim.Drops.Length);
        for (var i = 0; i < expected.Length; i++)
        {
            var (got, want) = (sim.Drops[i], expected[i]);
            Assert.Equal(want.Alive, got.Alive);
            Assert.Equal(want.Ink, got.Ink);
            Assert.Equal(want.X, got.X, Tolerance);
            Assert.Equal(want.Y, got.Y, Tolerance);
            Assert.Equal(want.Vx, got.Vx, Tolerance);
            Assert.Equal(want.Vy, got.Vy, Tolerance);
            Assert.Equal(want.R, got.R, Tolerance);
            Assert.Equal(want.Age, got.Age, Tolerance);
        }
    }
}
