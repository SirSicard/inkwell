// Architecture rule 9 on Windows, end to end through the real clock and the real pipeline (WARP):
// idle draws one still frame and then nothing, live draws at the compositor's rate, and the clock's
// thread exists only while something is live. The Mac's InkViewTests and InkClockTests.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

/// <summary>An offscreen host: the ink drawn into a texture, counted as presented.</summary>
internal sealed unsafe class OffscreenTarget : IInkTarget, IDisposable
{
    private InkTexture? texture;

    public bool Render(InkPipeline pipeline, in InkUniforms uniforms, InkMark? mark)
    {
        var w = (int)uniforms.ResX;
        var h = (int)uniforms.ResY;
        if (texture is null || texture.Width != w || texture.Height != h)
        {
            texture?.Dispose();
            texture = new InkTexture(pipeline, w, h);
        }
        pipeline.Encode(texture.View, w, h, uniforms, mark);
        pipeline.Context->Flush();
        return true;
    }

    public void ReleaseDeviceResources()
    {
        texture?.Dispose();
        texture = null;
    }

    public void SetFallback(bool shown)
    {
    }

    public string? CheckDevice() => null;

    public void Dispose() => texture?.Dispose();
}

public sealed class SurfaceTests(ITestOutputHelper output)
{
    private static readonly InkPipelineLoader Loader = new(() => new InkPipeline(TestPipeline.Adapter));

    private static (InkSurface Surface, OffscreenTarget Target) Make(InkClock clock, double width = 96, double height = 84)
    {
        Assert.Null(Loader.Wait().Failure);
        var target = new OffscreenTarget();
        var surface = new InkSurface(target, Loader, clock) { AssumeReduceMotion = false };
        var (w, h) = InkSurface.CanvasPixels(width, height, 2);
        surface.SetCanvas(w, h, width);
        return (surface, target);
    }

    [Fact]
    public void IdleDrawsOneStillFrameLiveRunsAtTheFrameRateAndIdleStopsAgain()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            var clock = new InkClock(ui.Post);
            var (surface, target) = Make(clock);
            using var _ = target;
            using var __ = surface;

            surface.SetOnScreen(true);
            Assert.Equal(1, surface.FramesDrawn);
            Assert.False(surface.IsAnimating);
            ui.Pump(0.5);
            Assert.Equal(1, surface.FramesDrawn);
            Assert.Equal(0, clock.ThreadCount);

            var before = InkFrames.Count;
            var started = InkClock.Now();
            surface.State = InkState.Meeting;
            Assert.True(surface.IsAnimating);
            Assert.Equal(1, clock.ThreadCount);
            ui.Pump(1.0);
            var live = surface.FramesDrawn - 1;
            var rate = live / (InkClock.Now() - started);
            output.WriteLine(FormattableString.Invariant($"live: {live} frames at {rate:F1} per second"));
            // About 60 a second at most; about 30 in a session without a compositor (SSH), where
            // the clock falls back to a 16 ms sleep on a 15.6 ms timer.
            Assert.InRange(rate, 25, 75);
            Assert.Equal(live, InkFrames.Count - before);

            surface.State = InkState.Idle;
            Assert.False(surface.IsAnimating);
            var settled = surface.FramesDrawn;
            ui.Pump(0.5);
            Assert.Equal(settled, surface.FramesDrawn);
            Assert.Equal(0, clock.ThreadCount);
        }
    }

    [Fact]
    public void NothingDrawsWhileHiddenAndAnimationEffectsOffDrawsStills()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            var clock = new InkClock(ui.Post);
            var (surface, target) = Make(clock);
            using var _ = target;
            using var __ = surface;

            surface.State = InkState.Dictating;
            ui.Pump(0.3);
            Assert.Equal(0, surface.FramesDrawn);
            Assert.Equal(0, clock.ThreadCount);

            surface.AssumeReduceMotion = true;
            surface.SetOnScreen(true);
            Assert.Equal(1, surface.FramesDrawn);
            surface.State = InkState.Meeting;
            Assert.Equal(2, surface.FramesDrawn);
            ui.Pump(0.3);
            Assert.Equal(2, surface.FramesDrawn);
            Assert.False(surface.IsAnimating);
            Assert.Equal(0, clock.ThreadCount);
        }
    }

    [Fact]
    public void TwoLiveSurfacesShareOneClockThread()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            var clock = new InkClock(ui.Post);
            var (drop, t1) = Make(clock);
            var (rail, t2) = Make(clock, 56, 700);
            using var _ = t1;
            using var __ = t2;
            using var ___ = drop;
            using var ____ = rail;
            drop.SetOnScreen(true);
            rail.SetOnScreen(true);
            drop.State = InkState.Meeting;
            rail.State = InkState.Meeting;
            Assert.Equal(2, clock.ClientCount);
            Assert.Equal(1, clock.ThreadCount);
            ui.Pump(0.5);
            Assert.True(drop.FramesDrawn > 10);
            Assert.True(rail.FramesDrawn > 10);
            drop.State = InkState.Idle;
            Assert.Equal(1, clock.ClientCount);
            rail.SetOnScreen(false);
            Assert.Equal(0, clock.ClientCount);
            ui.Pump(0.3);
            Assert.Equal(0, clock.ThreadCount);
        }
    }

    [Fact]
    public void ASurfaceMadeBeforeThePipelineDrawsWhenItArrives()
    {
        var ui = new UiThread();
        var clock = new InkClock(ui.Post);
        using var gate = new ManualResetEventSlim();
        var loader = new InkPipelineLoader(() =>
        {
            gate.Wait();
            return new InkPipeline(InkAdapter.Warp);
        });
        using var target = new OffscreenTarget();
        using var surface = new InkSurface(target, loader, clock) { AssumeReduceMotion = false };
        surface.SetCanvas(96, 84, 96);
        surface.SetOnScreen(true);
        Assert.False(surface.IsReady);
        Assert.Equal(0, surface.FramesDrawn);
        gate.Set();
        var until = DateTime.UtcNow.AddSeconds(10);
        while (!surface.IsReady && DateTime.UtcNow < until)
        {
            ui.Pump(0.05);
        }
        Assert.True(surface.IsReady);
        Assert.Equal(1, surface.FramesDrawn);
        loader.Outcome!.Pipeline!.Dispose();
    }

    [Fact]
    public void AFailedCompileLeavesPaperAndSaysWhy()
    {
        var ui = new UiThread();
        var clock = new InkClock(ui.Post);
        var loader = new InkPipelineLoader(() => new InkPipeline(InkAdapter.Warp, "not a shader"));
        Assert.StartsWith("couldn't compile the ink shader", loader.Wait().Failure, StringComparison.Ordinal);
        using var target = new OffscreenTarget();
        using var surface = new InkSurface(target, loader, clock);
        surface.SetCanvas(96, 84, 96);
        surface.SetOnScreen(true);
        surface.State = InkState.Dictating;
        ui.Pump(0.2);
        Assert.Equal(0, surface.FramesDrawn);
        Assert.NotNull(surface.Failure);
        Assert.Equal(0, clock.ThreadCount);
    }

    /// <summary>
    /// A host whose every frame outlasts the clock's interval several times over, as a Present
    /// does that waits on a compositor fallen behind: the clock always queues the next tick early
    /// in the frame.
    /// </summary>
    private sealed class SlowTarget : IInkTarget
    {
        public bool Render(InkPipeline pipeline, in InkUniforms uniforms, InkMark? mark)
        {
            Thread.Sleep(100);
            return true;
        }

        public void ReleaseDeviceResources()
        {
        }

        public void SetFallback(bool shown)
        {
        }

        public string? CheckDevice() => null;
    }

    /// <summary>
    /// With every frame longer than the clock's interval, the next tick is always queued before the
    /// last one ends. The tests' UI thread still hands back when its time is up: draining until
    /// nothing was queued, it never did, and the test using it never ended.
    /// </summary>
    [Fact]
    public void ThePumpEndsOnTimeWhileEveryFrameOutlastsTheClock()
    {
        lock (TestPipeline.Lock)
        {
            Assert.Null(Loader.Wait().Failure);
            var ui = new UiThread();
            var clock = new InkClock(ui.Post, () =>
            {
                Thread.Sleep(1);
                return true;
            });
            using var surface = new InkSurface(new SlowTarget(), Loader, clock) { AssumeReduceMotion = false };
            surface.SetCanvas(96, 84, 96);
            surface.SetOnScreen(true);
            surface.State = InkState.Meeting;
            Assert.True(surface.IsAnimating);

            // The pump runs on a thread of its own so that a pump that never returns fails this
            // test instead of hanging the run; this thread only waits meanwhile.
            var pump = new Thread(() => ui.Pump(0.3)) { IsBackground = true };
            pump.Start();
            Assert.True(pump.Join(TimeSpan.FromSeconds(30)), "the pump returned after its 0.3 s");
            Assert.True(surface.FramesDrawn > 1, $"the clock drew while it pumped: {surface.FramesDrawn} frames");

            surface.State = InkState.Idle;
            ui.Pump(0.1);
            Assert.Equal(0, clock.ThreadCount);
        }
    }

    [Fact]
    public void TheCanvasFollowsThePrototypesSizingRule()
    {
        Assert.Equal((192, 168), InkSurface.CanvasPixels(96, 84, 2));
        Assert.Equal((144, 126), InkSurface.CanvasPixels(96, 84, 1.5));
        Assert.Equal((192, 168), InkSurface.CanvasPixels(96, 84, 3));
        // A large zone is capped at 1.25.
        Assert.Equal((450, 900), InkSurface.CanvasPixels(360, 720, 2));
    }
}
