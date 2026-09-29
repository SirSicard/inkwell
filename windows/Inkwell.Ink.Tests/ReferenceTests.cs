// Direct3D against Metal: each state rendered here at the fixed time (t = 12, 360 x 720, wordmark
// on) is compared with the Mac's Metal render of the same state, written by the Mac's
// ReferenceTests (INK_RENDER_OUT, files metal-<state>.png and metal-<state>-reference-face.png).
// Skipped unless INK_METAL_DIR names a directory holding them. INK_RENDER_OUT writes this
// renderer's frames (d3d11-<state>.png) beside, for a person to look at.
//
// The wordmark's letters differ by design (Segoe UI Black here, SF Pro Black there), so the box
// they can occupy is left out of the strict checks: the union of this wordmark's box and the Mac
// letters' extent (where the Mac's two renders, SF Pro and Helvetica, differ), grown by 2 px.
// Criteria: SSIM over the whole frame at least 0.9 (the step's gate); outside the box, SSIM over
// the ink at least 0.95 and no pixel off by more than 8/255 (the S0.5 gate).
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class ReferenceTests(ITestOutputHelper output)
{
    [Fact]
    public void TheScorerOnKnownAnswers()
    {
        var rng = InkRandom.Seeded(7);
        const int w = 40, h = 30;
        var image = Enumerable.Range(0, w * h).Select(_ => 255 * rng.Next()).ToArray();
        Assert.Equal(1, Ssim.Compute(image, image, w, h).Mean, 1e-12);
        var a = Enumerable.Repeat(100.0, w * h).ToArray();
        var b = Enumerable.Repeat(120.0, w * h).ToArray();
        const double c1 = 0.01 * 255 * (0.01 * 255);
        Assert.Equal((2.0 * 100 * 120 + c1) / (100.0 * 100 + 120 * 120 + c1), Ssim.Compute(a, b, w, h).Mean, 1e-12);
        var inverted = image.Select(v => 255 - v).ToArray();
        Assert.True(Ssim.Compute(image, inverted, w, h).Mean < 0.2);
        Assert.Equal(Ssim.Compute(image, inverted, w, h).Mean, Ssim.Compute(inverted, image, w, h).Mean, 1e-12);
        Assert.Equal((w - 10) * (h - 10), Ssim.Compute(image, image, w, h).Values.Length);
    }

    [Fact]
    public void EveryStateMatchesTheMacsMetalRender()
    {
        var directory = Environment.GetEnvironmentVariable("INK_METAL_DIR");
        Assert.SkipWhen(string.IsNullOrEmpty(directory), "INK_METAL_DIR is not set");
        var outDir = Environment.GetEnvironmentVariable("INK_RENDER_OUT");
        if (!string.IsNullOrEmpty(outDir))
        {
            Directory.CreateDirectory(outDir);
        }
        output.WriteLine($"adapter: {TestPipeline.Get().AdapterName} (asked for {TestPipeline.Adapter})");
        foreach (var state in InkStates.All)
        {
            var metal = Png.Read(Path.Combine(directory!, $"metal-{state.Name()}.png"));
            var metalOtherFace = Png.Read(Path.Combine(directory!, $"metal-{state.Name()}-reference-face.png"));
            Assert.Equal(360, metal.Width);
            Assert.Equal(720, metal.Height);
            InkImage ours;
            lock (TestPipeline.Lock)
            {
                ours = InkSnapshot.Render(TestPipeline.Get(), state, 12, 360, 720);
            }
            if (!string.IsNullOrEmpty(outDir))
            {
                Png.Write(Path.Combine(outDir, $"d3d11-{state.Name()}.png"), ours.Width, ours.Height, ours.Rgba);
            }
            var box = ours.WordmarkBox!.Value.Union(Differing(metal, metalOtherFace)).Outset(2);
            var score = new Comparison(ours, metal, box);
            output.WriteLine($"{state.Name()} [{ours.WordmarkFont}] box {box}: {score}");
            Assert.True(score.WholeSsim >= 0.9, $"{state}: SSIM over the frame {score.WholeSsim}");
            Assert.True(score.InkSsimOutsideBox >= 0.95, $"{state}: SSIM over the ink {score.InkSsimOutsideBox}");
            Assert.True(score.OffOutsideBox == 0, $"{state}: {score.OffOutsideBox} pixels off by more than 8/255 outside the wordmark");
        }
    }

    /// <summary>The bounding box of the pixels where two renders differ: the Mac's letters in both faces.</summary>
    private static InkRect Differing(RgbImage a, RgbImage b)
    {
        int x0 = int.MaxValue, y0 = int.MaxValue, x1 = -1, y1 = -1;
        for (var y = 0; y < a.Height; y++)
        {
            for (var x = 0; x < a.Width; x++)
            {
                var i = (y * a.Width + x) * 3;
                if (a.Rgb[i] == b.Rgb[i] && a.Rgb[i + 1] == b.Rgb[i + 1] && a.Rgb[i + 2] == b.Rgb[i + 2])
                {
                    continue;
                }
                (x0, y0, x1, y1) = (Math.Min(x0, x), Math.Min(y0, y), Math.Max(x1, x), Math.Max(y1, y));
            }
        }
        Assert.True(x1 >= 0, "the Mac's two faces differ somewhere");
        return new InkRect(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
    }
}
