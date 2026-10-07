// Text over the main window's orb, as the Mac's OrbBehindTextTests: the orb sits behind every
// screen's text, so wherever it draws (its home and each corner of the region it wanders in), in
// every state, the mode's text keeps 4.5:1 and its secondary text 3:1 or more over it, with every
// preset in both modes. At rest that is the orb leaning toward the preset (GlowLook.ShellRestTint,
// 0.6) at 70 %; live, 30 %. Drawn with WARP and composited over the mode's background at the
// opacity the window gives it, as InkPanel does.
using Inkwell.Core.Glow;
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class OrbBehindTextTests(ITestOutputHelper output)
{
    private static readonly InkPipelineLoader Loader = new(() => new InkPipeline(TestPipeline.Adapter));

    /// <summary>WCAG's relative luminance of an sRGB colour, 0..1.</summary>
    private static double Luminance(double r, double g, double b)
    {
        static double Linear(double v) => v <= 0.04045 ? v / 12.92 : Math.Pow((v + 0.055) / 1.055, 2.4);
        return 0.2126 * Linear(r) + 0.7152 * Linear(g) + 0.0722 * Linear(b);
    }

    private static double Luminance(GlowColor c) => Luminance(c.Red, c.Green, c.Blue);

    private static double Contrast(double a, double b) => (Math.Max(a, b) + 0.05) / (Math.Min(a, b) + 0.05);

    private static (float, float, float) Tuple(GlowRgb c) => ((float)c.R, (float)c.G, (float)c.B);

    /// <summary>The main window's look for a preset in a mode, as GlowTheme makes it.</summary>
    private static GlowLook Look(bool dark, string preset)
    {
        var c = GlowScheme.Resolve(dark, preset);
        return new GlowLook(dark, Tuple(c.You), Tuple(c.YouPartner), Tuple(c.Them), Tuple(c.ThemPartner), Tuple(c.Idle), Tuple(c.Ink), GlowLook.ShellRestTint);
    }

    /// <summary>The rest tint travels in idle's fourth lane, clamped; the uniform block keeps its size.</summary>
    [Fact]
    public void TheRestTintTravelsInIdlesFourthLane()
    {
        var simulation = new InkSimulation(InkRandom.Seeded(1)) { CanvasWidth = 100, CanvasHeight = 100 };
        Assert.Equal(0, simulation.Uniforms(InkPlacement.Centre, GlowLook.Default, moving: false).Idle.W);
        Assert.Equal(0.6f, simulation.Uniforms(InkPlacement.Centre, Look(false, "indigo"), moving: false).Idle.W);
        Assert.Equal(1, simulation.Uniforms(InkPlacement.Centre, GlowLook.Default with { RestTint = 3 }, moving: false).Idle.W);
        Assert.Equal(160, System.Runtime.InteropServices.Marshal.SizeOf<InkUniforms>());
    }

    [Theory]
    [InlineData(0.1f, 0.6f, 0f)]
    [InlineData(0.7f, 0.6f, 0f)]
    [InlineData(0.85f, 0.8f, 0.5f)]
    [InlineData(1f, 1f, 1f)]
    public void ShellStrengthAboveSeventyRaisesTintAndBoost(float strength, float tint, float boost)
    {
        var look = GlowLook.Default.WithShellStrength(strength);
        Assert.Equal(tint, look.RestTint, 5);
        Assert.Equal(boost, look.RestBoost, 5);
        var simulation = new InkSimulation(InkRandom.Seeded(1));
        Assert.Equal(boost, simulation.Uniforms(InkPlacement.Centre, look, moving: false).Ink.W, 5);
        Assert.Equal(0, simulation.Uniforms(InkPlacement.Centre, GlowLook.Default, moving: false).Ink.W);
        Assert.Equal(1, simulation.Uniforms(InkPlacement.Centre, look with { RestBoost = 3 }, moving: false).Ink.W);
        Assert.Equal(160, System.Runtime.InteropServices.Marshal.SizeOf<InkUniforms>());
    }

    /// <summary>The home is inside the region the orb wanders in.</summary>
    [Fact]
    public void TheHomeIsInsideTheWander() =>
        Assert.True(OrbWander.Main.Contains((GlowTokens.Orb.Main.X, GlowTokens.Orb.Main.Y)));

    [Fact]
    public void TextStaysReadableOverTheMainWindowsOrbWithEveryPresetInBothModes()
    {
        lock (TestPipeline.Lock)
        {
            var outcome = Loader.Wait();
            Assert.Null(outcome.Failure);
            var pipeline = outcome.Pipeline!;
            var home = GlowTokens.Orb.Main;
            var placements = new List<InkPlacement> { new(home.X, home.Y, home.Unit) };
            placements.AddRange(OrbWander.Main.Corners.Select(c => new InkPlacement(c.X, c.Y, home.Unit)));
            var (worstText, worstSecondary) = (double.PositiveInfinity, double.PositiveInfinity);
            var worstAt = "";
            foreach (var dark in new[] { false, true })
            {
                var mode = GlowScheme.Palette(dark);
                var (br, bg, bb) = (mode.Background.Red, mode.Background.Green, mode.Background.Blue);
                var text = Luminance(mode.Text);
                var secondary = Luminance(mode.Secondary);
                foreach (var preset in GlowScheme.Presets)
                {
                    var look = Look(dark, preset.Id);
                    foreach (var state in Enum.GetValues<InkState>())
                    {
                        var opacity = (double)OrbFade.BehindText(state.IsLive(), dimmed: false);
                        foreach (var placement in placements)
                        {
                            var drawn = 0;
                            foreach (var t in new[] { 3.0, 12, 27 })
                            {
                                // The orb scales with the window, so a small canvas holds the same colours.
                                var image = InkSnapshot.Render(pipeline, state, t, 208, 140, InkVoice.Levels(1, 1), look, placement, blotDepth: 0.45);
                                for (var i = 0; i < image.Rgba.Length; i += 4)
                                {
                                    if (image.Rgba[i + 3] == 0)
                                    {
                                        continue;
                                    }
                                    var alpha = image.Rgba[i + 3] / 255.0;
                                    // Premultiplied over the background, as the window composites it.
                                    var shown = Luminance(
                                        image.Rgba[i] / 255.0 * opacity + br * (1 - alpha * opacity),
                                        image.Rgba[i + 1] / 255.0 * opacity + bg * (1 - alpha * opacity),
                                        image.Rgba[i + 2] / 255.0 * opacity + bb * (1 - alpha * opacity));
                                    var label = $"{(dark ? "dark" : "light")} {preset.Id} {state} at ({placement.X}, {placement.Y})";
                                    var (ct, cs) = (Contrast(text, shown), Contrast(secondary, shown));
                                    Assert.True(ct >= 4.5, $"{label}: text {ct:F2}:1");
                                    Assert.True(cs >= 3, $"{label}: secondary {cs:F2}:1");
                                    if (cs < worstSecondary)
                                    {
                                        worstSecondary = cs;
                                        worstAt = label;
                                    }
                                    worstText = Math.Min(worstText, ct);
                                    drawn++;
                                }
                            }
                            Assert.True(drawn > 300, $"{(dark ? "dark" : "light")} {preset.Id} {state}: the orb is in the frame");
                        }
                    }
                }
            }
            output.WriteLine(FormattableString.Invariant($"worst text {worstText:F2}:1; worst secondary {worstSecondary:F2}:1 ({worstAt})"));
        }
    }
}
