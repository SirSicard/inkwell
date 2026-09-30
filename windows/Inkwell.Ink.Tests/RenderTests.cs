// The ink drawn offscreen through the real pipeline: the embedded HLSL compiled at run time with
// D3DCompile, as the app does, drawn by Direct3D 11 (WARP unless INK_TEST_ADAPTER=hardware). The
// Mac's RenderTests, for Windows.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class RenderTests
{
    private static InkImage Render(InkState state, bool wordmark = true, double cy = 0.5)
    {
        lock (TestPipeline.Lock)
        {
            return InkSnapshot.Render(TestPipeline.Get(), state, 12, 360, 720, wordmark: wordmark, cy: cy);
        }
    }

    [Fact]
    public void TheEmbeddedShaderIsFoundAndCompiles()
    {
        var source = InkPipeline.ShaderSource();
        Assert.StartsWith("// Generated from shaders/ink.wgsl", source, StringComparison.Ordinal);
        using var pipeline = new InkPipeline(InkAdapter.Warp, source);
        Assert.True(pipeline.IsWarp);
    }

    [Fact]
    public void ABrokenShaderIsAnErrorNotACrash()
    {
        var e = Assert.Throws<InkRendererException>(() => new InkPipeline(InkAdapter.Warp, "this is not HLSL"));
        Assert.StartsWith("couldn't compile the ink shader", e.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void EveryStateRendersTheSameTwiceAndDiffersFromTheOthers()
    {
        var images = new Dictionary<InkState, InkImage>();
        foreach (var state in InkStates.All)
        {
            var first = Render(state);
            var second = Render(state);
            Assert.Equal(first.Rgba, second.Rgba);
            images[state] = first;
        }
        foreach (var a in InkStates.All)
        {
            foreach (var b in InkStates.All.Where(b => b > a))
            {
                Assert.NotEqual(images[a].Rgba, images[b].Rgba);
            }
        }
    }

    [Fact]
    public void TheFarEndsInkAppearsOnlyWithTwoStreams()
    {
        Assert.Equal(0, SepiaPixels(Render(InkState.Dictating)));
        Assert.Equal(0, SepiaPixels(Render(InkState.Idle)));
        Assert.True(SepiaPixels(Render(InkState.Meeting)) > 500);
    }

    [Fact]
    public void ASilentFarEndIsAGhostWithASealRedOutline()
    {
        var problem = Render(InkState.Problem);
        Assert.True(SepiaPixels(problem) < SepiaPixels(Render(InkState.Meeting)) / 4);
        var seal = 0;
        for (var y = 0; y < problem.Height; y++)
        {
            for (var x = 0; x < problem.Width; x++)
            {
                var (r, g, b) = problem.Pixel(x, y);
                if (r > 150 && g < 120 && b < 110)
                {
                    seal++;
                }
            }
        }
        Assert.True(seal > 50, $"the dashed outline ({seal} px)");
    }

    /// <summary>The wordmark is knocked out of whatever sits under it: dark on paper, light on ink.</summary>
    [Fact]
    public void TheWordmarkIsDarkOnPaperAndLightOnInk()
    {
        byte[] coverage;
        InkRect box;
        lock (TestPipeline.Lock)
        {
            using var mark = Wordmark.Rasterize(TestPipeline.Get(), 360, 720, 360);
            coverage = mark.Mark.ReadCoverage(TestPipeline.Get());
            box = mark.Box;
            Assert.False(mark.Simulated, $"{mark.FontName} is a real face, not a synthesised bold");
        }
        var image = Render(InkState.Dictating, cy: 0.9);
        var plain = Render(InkState.Dictating, wordmark: false, cy: 0.9);
        int onPaper = 0, onInk = 0, full = 0;
        for (var y = 0; y < image.Height; y++)
        {
            for (var x = 0; x < image.Width; x++)
            {
                if (coverage[y * image.Width + x] != 255)
                {
                    continue;
                }
                full++;
                Assert.True(box.Outset(1).Contains(x + 0.5, y + 0.5), $"a letter pixel ({x}, {y}) inside the box {box}");
                var under = plain.Pixel(x, y);
                var letter = image.Pixel(x, y);
                Assert.InRange(255 - under.R - letter.R, -1, 1);
                Assert.InRange(255 - under.G - letter.G, -1, 1);
                Assert.InRange(255 - under.B - letter.B, -1, 1);
                var lum = plain.Luminance(x, y);
                if (lum > 180)
                {
                    onPaper++;
                }
                else if (lum < 60)
                {
                    onInk++;
                }
            }
        }
        Assert.True(full > 1000, $"the letters cover {full} px");
        Assert.True(onPaper > 100, $"letters on paper: {onPaper}");
        Assert.True(onInk > 100, $"letters on ink: {onInk}");
    }

    [Fact]
    public void TheWordmarkSitsWhereThePrototypePutsIt()
    {
        lock (TestPipeline.Lock)
        {
            using var mark = Wordmark.Rasterize(TestPipeline.Get(), 360, 720, 360);
            // round(360 * 0.158) = 57 px, x = round(57 * 0.62) = 35, top = round(57 * 0.7) = 40.
            Assert.Equal(57, mark.FontSize);
            Assert.Equal(35, mark.X);
            Assert.InRange(mark.BaselineFromTop, 80, 100);
            Assert.InRange(mark.TextWidth, 150, 300);
        }
    }

    /// <summary>The ink stays inside its zone (the rail, a Drop-sized zone): never nearer the edge than half the fence band.</summary>
    [Fact]
    public void TheInkStaysInsideItsZone()
    {
        foreach (var (w, h) in new[] { (112, 1400), (168, 168) })
        {
            var keepOut = (int)(0.05 * Math.Min(w, h)) - 1;
            var nearest = int.MaxValue;
            foreach (var state in new[] { InkState.Dictating, InkState.Meeting, InkState.Problem })
            {
                for (var t = 3.0; t <= 60; t += 3)
                {
                    InkImage image;
                    lock (TestPipeline.Lock)
                    {
                        image = InkSnapshot.Render(TestPipeline.Get(), state, t, w, h, wordmark: false, voice: InkVoice.Levels(1, 1));
                    }
                    for (var y = 0; y < h; y++)
                    {
                        for (var x = 0; x < w; x++)
                        {
                            if (image.Luminance(x, y) >= 170)
                            {
                                continue;
                            }
                            var d = Math.Min(Math.Min(x, w - 1 - x), Math.Min(y, h - 1 - y));
                            nearest = Math.Min(nearest, d);
                            Assert.True(d >= keepOut, $"{state} t={t} {w}x{h}: ink at ({x}, {y})");
                        }
                    }
                }
            }
            Assert.True(nearest < keepOut + (int)(0.05 * Math.Min(w, h)) + 2, $"{w}x{h}: nearest ink {nearest} px");
        }
    }

    private static int SepiaPixels(InkImage image)
    {
        var n = 0;
        for (var y = 0; y < image.Height; y++)
        {
            for (var x = 0; x < image.Width; x++)
            {
                var (r, g, b) = image.Pixel(x, y);
                if (r > 80 && r < 180 && r - g > 25 && g - b > 20)
                {
                    n++;
                }
            }
        }
        return n;
    }
}
