// The orb drawn offscreen through the real pipeline: the embedded HLSL compiled at run time with
// D3DCompile, as the app does, drawn by Direct3D 11 (WARP unless INK_TEST_ADAPTER=hardware). The
// Mac's RenderTests, for Windows.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class RenderTests
{
    private static InkImage Render(InkState state, GlowLook? look = null)
    {
        lock (TestPipeline.Lock)
        {
            return InkSnapshot.Render(TestPipeline.Get(), state, 12, 360, 720, look: look);
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

    /// <summary>
    /// Premultiplied, and transparent wherever there is no orb: the window's background and the
    /// Drop's pill show round it.
    /// </summary>
    [Fact]
    public void TheOrbIsPremultipliedAndTransparentAroundIt()
    {
        foreach (var state in InkStates.All)
        {
            var image = Render(state);
            Assert.Equal(0, image.Alpha(0, 0));
            Assert.Equal(0, image.Alpha(image.Width - 1, image.Height - 1));
            Assert.True(image.Alpha(image.Width / 2, image.Height / 2) > 0, $"{state}: the orb at the centre");
            for (var i = 0; i < image.Rgba.Length; i += 4)
            {
                var a = image.Rgba[i + 3];
                Assert.True(image.Rgba[i] <= a + 1 && image.Rgba[i + 1] <= a + 1 && image.Rgba[i + 2] <= a + 1, $"{state}: premultiplied at {i / 4}");
            }
        }
    }

    /// <summary>
    /// Over a backdrop (a SwapChainPanel's, which shows nothing behind it): opaque everywhere, the
    /// backdrop where there is no orb, and the orb blended over it where there is.
    /// </summary>
    [Fact]
    public void OverABackdropTheOrbIsBlendedAndTheRestIsTheBackdrop()
    {
        var day = (0xFB / 255f, 0xF8 / 255f, 0xF4 / 255f);
        InkImage bare, over;
        lock (TestPipeline.Lock)
        {
            bare = InkSnapshot.Render(TestPipeline.Get(), InkState.Dictating, 12, 360, 720);
            over = InkSnapshot.Render(TestPipeline.Get(), InkState.Dictating, 12, 360, 720, backdrop: day);
        }
        Assert.Equal((0xFB, 0xF8, 0xF4), ((int, int, int))over.Pixel(0, 0));
        Assert.Equal((0xFB, 0xF8, 0xF4), ((int, int, int))over.Pixel(over.Width - 1, over.Height - 1));
        for (var i = 0; i < over.Rgba.Length; i += 4)
        {
            Assert.Equal(255, over.Rgba[i + 3]);
        }
        // The centre: the orb's premultiplied colour plus the backdrop's share of what it leaves.
        var x = bare.Width / 2;
        var y = bare.Height / 2;
        var a = bare.Alpha(x, y) / 255.0;
        Assert.True(a > 0);
        var (r, _, _) = bare.Pixel(x, y);
        Assert.InRange(over.Pixel(x, y).R, r + 0xFB * (1 - a) - 2, r + 0xFB * (1 - a) + 2);
    }

    /// <summary>
    /// A frame the compositor has no room for yet is dropped, not waited for: the swapchain presents
    /// without waiting (DXGI_PRESENT_DO_NOT_WAIT), and "still drawing" is not a failure. Waiting
    /// there blocked the UI thread a whole frame per present, 60 times a second on a 60 Hz display.
    /// </summary>
    [Fact]
    public void APresentTheCompositorIsNotReadyForIsDroppedNotAFailure()
    {
        lock (TestPipeline.Lock)
        {
            using var swapChain = new CompositionSwapChain(TestPipeline.Get(), 64, 64);
            swapChain.InjectedPresentResult = CompositionSwapChain.WasStillDrawing;
            swapChain.Present();
            Assert.Equal(1, swapChain.DroppedFrames);
            swapChain.InjectedPresentResult = FlakyTarget.DeviceRemoved;
            Assert.Throws<InkRendererException>(swapChain.Present); // a lost device still is
        }
    }

    /// <summary>The orb takes the colours it is given: your colour while dictating, the idle colour at rest.</summary>
    [Fact]
    public void TheOrbTakesItsColours()
    {
        var red = (1f, 0f, 0f);
        var green = (0f, 1f, 0f);
        var look = GlowLook.Default with { YouA = red, YouB = red, Idle = green };
        var dictating = Render(InkState.Dictating, look);
        var idle = Render(InkState.Idle, look);
        var (r, g, _) = dictating.Pixel(dictating.Width / 2, dictating.Height / 2);
        Assert.True(r > g, $"dictating is yours ({r}, {g})");
        (r, g, _) = idle.Pixel(idle.Width / 2, idle.Height / 2);
        Assert.True(g > r, $"at rest the idle colour ({r}, {g})");
    }
}
