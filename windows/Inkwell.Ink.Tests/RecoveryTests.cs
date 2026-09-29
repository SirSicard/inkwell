// A lost device is not the end of the ink: the frame that fails (a TDR, a driver update, DWM
// restarting, a failed present) makes the surface release the host's device objects, show the
// host's fallback, ask for a new pipeline and try again with backoff (0.5, 1, 2 s, then every
// 5 s). The failure is injected through the host seam (a target whose frame throws what a lost
// device throws) and the loader's factory (a device that will not come back).
using Inkwell.Ink;
using TerraFX.Interop.Windows;
using Xunit;

namespace Inkwell.Ink.Tests;

/// <summary>An offscreen host whose next frame can fail as a lost device's does.</summary>
internal sealed class FlakyTarget : IInkTarget, IDisposable
{
    private readonly OffscreenTarget inner = new();

    /// <summary>DXGI_ERROR_DEVICE_REMOVED (winerror.h).</summary>
    public const int DeviceRemoved = unchecked((int)0x887A0005);

    public bool FailNext { get; set; }

    /// <summary>What the failing frame throws: a lost device by default.</summary>
    public int FailWith { get; set; } = DeviceRemoved;

    /// <summary>DXGI_ERROR_NOT_CURRENTLY_AVAILABLE (winerror.h): no swapchain now, the device fine.</summary>
    public const int NotCurrentlyAvailable = unchecked((int)0x887A0022);
    public int Releases { get; private set; }
    public List<bool> Fallbacks { get; } = [];
    public bool Fallback => Fallbacks.Count > 0 && Fallbacks[^1];

    public bool Render(InkPipeline pipeline, in InkUniforms uniforms, InkMark? mark)
    {
        if (FailNext)
        {
            FailNext = false;
            throw new InkRendererException("present the ink", (HRESULT)FailWith);
        }
        return inner.Render(pipeline, uniforms, mark);
    }

    public void ReleaseDeviceResources()
    {
        Releases++;
        inner.ReleaseDeviceResources();
    }

    public void SetFallback(bool shown) => Fallbacks.Add(shown);

    /// <summary>What the host's own device check reports (null: fine).</summary>
    public string? DeviceProblem { get; set; }

    public string? CheckDevice() => DeviceProblem;

    public void Dispose() => inner.Dispose();
}

public sealed class RecoveryTests
{
    /// <summary>A surface on its own loader, with retries captured instead of timed.</summary>
    private sealed class Rig : IDisposable
    {
        public readonly UiThread Ui = new();
        public readonly FlakyTarget Target = new();
        public readonly InkPipelineLoader Loader;
        public readonly InkSurface Surface;
        public readonly List<TimeSpan> Delays = [];
        public readonly List<string?> Failures = [];
        private readonly Queue<Action> retries = new();
        private readonly Queue<Action> checks = new();

        public Rig(Func<InkPipeline> make)
        {
            Loader = new InkPipelineLoader(make);
            Surface = new InkSurface(Target, Loader, new InkClock(Ui.Post))
            {
                AssumeReduceMotion = true,
                HealthScheduler = (_, action) => checks.Enqueue(action),
                WatchesDevice = true,
            };
            Surface.Scheduler = (delay, action) =>
            {
                Delays.Add(delay);
                retries.Enqueue(action);
            };
            Surface.FailureChanged += Failures.Add;
            Surface.SetCanvas(96, 84, 96);
            Surface.SetOnScreen(true);
        }

        /// <summary>Runs the pending retry, then pumps until the loader has answered.</summary>
        public void Retry()
        {
            Assert.NotEmpty(retries);
            var compiles = Loader.Compiles;
            retries.Dequeue()();
            var until = DateTime.UtcNow.AddSeconds(20);
            while (Loader.Compiles == compiles && DateTime.UtcNow < until)
            {
                Ui.Pump(0.02);
            }
            Ui.Pump(0.1);
        }

        public void PumpUntil(Func<bool> done)
        {
            var until = DateTime.UtcNow.AddSeconds(20);
            while (!done() && DateTime.UtcNow < until)
            {
                Ui.Pump(0.02);
            }
        }

        public int PendingRetries => retries.Count;

        public int PendingChecks => checks.Count;

        /// <summary>Runs the pending device check.</summary>
        public void RunCheck()
        {
            Assert.NotEmpty(checks);
            checks.Dequeue()();
        }

        /// <summary>Runs the pending retry on this thread (a retry that needs no compile).</summary>
        public void RunRetry()
        {
            Assert.NotEmpty(retries);
            retries.Dequeue()();
        }

        public void Dispose()
        {
            Surface.Dispose();
            Target.Dispose();
            Loader.Outcome?.Pipeline?.Dispose();
        }
    }

    [Fact]
    public void ALostDeviceIsRecreatedAndTheInkDrawsAgain()
    {
        using var rig = new Rig(() => new InkPipeline(InkAdapter.Warp));
        rig.PumpUntil(() => rig.Surface.FramesDrawn == 1);
        Assert.Equal(1, rig.Surface.FramesDrawn);
        var first = rig.Loader.Outcome!.Pipeline;

        rig.Target.FailNext = true;
        rig.Surface.State = InkState.Meeting; // a still frame (motion off): it fails
        Assert.Equal(1, rig.Surface.FramesDrawn);
        Assert.StartsWith("couldn't present the ink (0x887A0005)", rig.Surface.Failure, StringComparison.Ordinal);
        Assert.Equal(1, rig.Target.Releases);
        Assert.True(rig.Target.Fallback, "the host shows its fallback while the ink cannot draw");
        Assert.Equal([TimeSpan.FromSeconds(0.5)], rig.Delays);

        rig.Retry();
        rig.PumpUntil(() => rig.Surface.FramesDrawn == 2);
        Assert.Equal(2, rig.Loader.Compiles);
        Assert.NotSame(first, rig.Loader.Outcome!.Pipeline);
        Assert.Equal(2, rig.Surface.FramesDrawn);
        Assert.Null(rig.Surface.Failure);
        Assert.False(rig.Target.Fallback, "drawing again: the fallback goes");
        Assert.Equal(2, rig.Failures.Count);
        Assert.Null(rig.Failures[1]);
    }

    [Fact]
    public void WhenTheDeviceWillNotComeBackTheFallbackStaysAndRetriesBackOff()
    {
        var makes = 0;
        var deviceBack = false;
        using var rig = new Rig(() =>
        {
            if (++makes > 1 && !Volatile.Read(ref deviceBack))
            {
                throw new InkRendererException("make a Direct3D 11 device", (HRESULT)FlakyTarget.DeviceRemoved);
            }
            return new InkPipeline(InkAdapter.Warp);
        });
        rig.PumpUntil(() => rig.Surface.FramesDrawn == 1);
        rig.Target.FailNext = true;
        rig.Surface.Invalidate();
        Assert.True(rig.Target.Fallback);

        for (var i = 0; i < 5; i++)
        {
            rig.Retry();
            Assert.True(rig.Target.Fallback, $"attempt {i + 1}: still the fallback");
            Assert.StartsWith("couldn't make a Direct3D 11 device", rig.Surface.Failure, StringComparison.Ordinal);
        }
        Assert.Equal([0.5, 1, 2, 5, 5, 5], rig.Delays.Select(d => d.TotalSeconds));
        Assert.Equal(1, rig.Surface.FramesDrawn);

        Volatile.Write(ref deviceBack, true);
        rig.Retry();
        rig.PumpUntil(() => rig.Surface.FramesDrawn == 2);
        Assert.Equal(2, rig.Surface.FramesDrawn);
        Assert.False(rig.Target.Fallback);
        Assert.Null(rig.Surface.Failure);
    }

    /// <summary>A failure that is not the device's (a swapchain DWM will not give now) keeps the pipeline: retries make only the host's objects again, and it is announced once.</summary>
    [Fact]
    public void AHostFailureIsRetriedWithoutANewPipeline()
    {
        using var rig = new Rig(() => new InkPipeline(InkAdapter.Warp));
        rig.PumpUntil(() => rig.Surface.FramesDrawn == 1);
        rig.Target.FailWith = FlakyTarget.NotCurrentlyAvailable;
        rig.Target.FailNext = true;
        rig.Surface.Invalidate();
        Assert.True(rig.Target.Fallback);
        Assert.Equal(1, rig.Target.Releases);

        // It keeps failing for a while: the same failure, said once, and no compile.
        for (var i = 0; i < 4; i++)
        {
            rig.Target.FailNext = true;
            rig.RunRetry();
            Assert.True(rig.Target.Fallback);
        }
        Assert.Equal([0.5, 1, 2, 5, 5], rig.Delays.Select(d => d.TotalSeconds));
        Assert.Single(rig.Failures);
        Assert.Equal(1, rig.Loader.Compiles);

        rig.RunRetry();
        Assert.Equal(2, rig.Surface.FramesDrawn);
        Assert.False(rig.Target.Fallback);
        Assert.Null(rig.Surface.Failure);
        Assert.Equal(1, rig.Loader.Compiles);
    }

    /// <summary>
    /// Animation effects off and a still frame on screen: nothing draws, yet a DWM restart (the
    /// host's composition device lost) is noticed by the once-a-second check, the fallback shows,
    /// and the retry draws again.
    /// </summary>
    [Fact]
    public void ALostCompositionIsNoticedWithAnimationOffAndNothingDrawing()
    {
        using var rig = new Rig(() => new InkPipeline(InkAdapter.Warp));
        rig.PumpUntil(() => rig.Surface.FramesDrawn == 1);
        rig.Surface.State = InkState.Meeting;
        Assert.Equal(2, rig.Surface.FramesDrawn);
        Assert.False(rig.Surface.IsAnimating);

        Assert.Equal(1, rig.PendingChecks);
        rig.RunCheck();
        Assert.Equal(2, rig.Surface.FramesDrawn);
        Assert.Equal(1, rig.PendingChecks);

        rig.Target.DeviceProblem = "couldn't keep the Drop's composition device (DWM restarted?)";
        rig.RunCheck();
        Assert.Equal(rig.Target.DeviceProblem, rig.Surface.Failure);
        Assert.True(rig.Target.Fallback);
        Assert.Equal(1, rig.Target.Releases);
        Assert.Equal(0, rig.PendingChecks);
        Assert.Equal([TimeSpan.FromSeconds(0.5)], rig.Delays);

        rig.Target.DeviceProblem = null;
        rig.RunRetry();
        Assert.Equal(3, rig.Surface.FramesDrawn);
        Assert.False(rig.Target.Fallback);
        Assert.Null(rig.Surface.Failure);
        Assert.Equal(1, rig.Loader.Compiles);
        Assert.Equal(1, rig.PendingChecks);

        // Hidden: the pending check finds nothing to do and none follows.
        rig.Surface.SetOnScreen(false);
        rig.RunCheck();
        Assert.Equal(0, rig.PendingChecks);
    }

    [Fact]
    public void AShaderThatDoesNotCompileShowsTheFallbackAndIsNotRetried()
    {
        using var rig = new Rig(() => new InkPipeline(InkAdapter.Warp, "not a shader"));
        rig.PumpUntil(() => rig.Surface.Failure is not null);
        Assert.StartsWith("couldn't compile the ink shader", rig.Surface.Failure, StringComparison.Ordinal);
        Assert.True(rig.Target.Fallback);
        Assert.Empty(rig.Delays);
        Assert.Equal(0, rig.Surface.FramesDrawn);
        Assert.Single(rig.Failures);
    }

    [Fact]
    public void AHiddenSurfaceWaitsForItsNextShowToRetry()
    {
        using var rig = new Rig(() => new InkPipeline(InkAdapter.Warp));
        rig.PumpUntil(() => rig.Surface.FramesDrawn == 1);
        rig.Target.FailNext = true;
        rig.Surface.Invalidate();
        rig.Surface.SetOnScreen(false);
        Assert.False(rig.Target.Fallback, "hidden: no fallback either");

        rig.Retry();
        Assert.Equal(1, rig.Loader.Compiles);
        Assert.Equal(0, rig.PendingRetries);

        rig.Surface.SetOnScreen(true);
        Assert.True(rig.Target.Fallback);
        Assert.Equal(1, rig.PendingRetries);
        rig.Retry();
        rig.PumpUntil(() => rig.Surface.FramesDrawn == 2);
        Assert.Equal(2, rig.Surface.FramesDrawn);
        Assert.Null(rig.Surface.Failure);
    }
}
