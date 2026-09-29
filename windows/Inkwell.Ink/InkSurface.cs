// One ink on screen: what the Mac's InkView does, apart from AppKit. A host (the Drop's window, a
// SwapChainPanel) owns the pixels and says when it is visible and how big it is; the surface keeps
// the state, the simulation and the schedule, and asks the host to present a frame only when the
// schedule says so: on the shared clock while live, one still frame otherwise, nothing while
// hidden. Every frame presented is counted (InkFrames). UI thread only.
//
// The canvas follows the prototype's sizing rule, as on the Mac: the display scale, capped at 1.25
// for a large canvas (over 180,000 square DIPs) and at 2 otherwise.
namespace Inkwell.Ink;

/// <summary>What an ink surface draws into.</summary>
public interface IInkTarget
{
    /// <summary>
    /// Draws one frame with <paramref name="uniforms"/> into the host's pixels and presents it.
    /// False when nothing could be presented (the host is not ready). Throws
    /// <see cref="InkRendererException"/> when the device fails.
    /// </summary>
    bool Render(InkPipeline pipeline, in InkUniforms uniforms, InkMark? mark);
}

/// <summary>One ink on screen, drawn into a host.</summary>
public sealed class InkSurface : IDisposable
{
    private readonly IInkTarget target;
    private readonly InkClock clock;
    private readonly InkSimulation simulation = new();
    private InkSchedule schedule;
    private InkPipeline? pipeline;
    private Wordmark? wordmark;
    /// <summary>The wordmark could not be made for this canvas: drawn without it until the canvas or the setting changes.</summary>
    private bool wordmarkFailed;
    private bool hostOnScreen;
    private bool followsSystem = true;
    private bool disposed;
    private double lastTick;
    private bool firstTick = true;
    private (int Width, int Height, double PointWidth) canvas;

    /// <summary>A surface drawing into <paramref name="target"/> with the loader's pipeline, driven by <paramref name="clock"/> while live. UI thread.</summary>
    public InkSurface(IInkTarget target, InkPipelineLoader loader, InkClock clock)
    {
        ArgumentNullException.ThrowIfNull(loader);
        ArgumentNullException.ThrowIfNull(clock);
        this.target = target;
        this.clock = clock;
        // The prototype starts each ink at a random point in its slow motion.
        simulation.T = Random.Shared.NextDouble() * 30;
        schedule.SetReduceMotion(!SystemMotion.AnimationsEnabled);
        SystemMotion.Changed += MotionChanged;
        if (loader.Outcome is { } outcome)
        {
            Adopt(outcome);
        }
        else
        {
            loader.WhenReady(clock.Post, Adopt);
        }
    }

    /// <summary>What the ink shows.</summary>
    public InkState State
    {
        get => schedule.State;
        set
        {
            if (value == schedule.State)
            {
                return;
            }
            simulation.State = value;
            Perform(schedule.SetState(value));
        }
    }

    /// <summary>Whether the INKWELL wordmark is knocked out of the ink.</summary>
    public bool ShowsWordmark
    {
        get;
        set
        {
            if (field == value)
            {
                return;
            }
            field = value;
            DropWordmark();
            Perform(schedule.Invalidate());
        }
    }

    /// <summary>The ink body's centre height, 0..1 from the bottom.</summary>
    public double InkCentreHeight
    {
        get => simulation.Cy;
        set
        {
            if (value == simulation.Cy)
            {
                return;
            }
            simulation.Cy = value;
            Perform(schedule.Invalidate());
        }
    }

    /// <summary>The live levels, read once per frame while live; null reads silence. Must return at once.</summary>
    public Func<InkLevels>? Levels { get; set; }

    /// <summary>Frames this surface has presented.</summary>
    public int FramesDrawn { get; private set; }

    /// <summary>Whether the surface is on the clock (live, on screen, motion allowed).</summary>
    public bool IsAnimating => clock.Contains(this);

    /// <summary>Whether the pipeline has arrived. Until then the host shows its paper and no clock runs.</summary>
    public bool IsReady => pipeline is not null;

    /// <summary>Why the ink cannot draw, if it cannot.</summary>
    public string? Failure { get; private set; }

    /// <summary>The canvas in pixels.</summary>
    public (int Width, int Height) Canvas => (canvas.Width, canvas.Height);

    /// <summary>The pipeline, once it has arrived (hosts make their swapchains on its device).</summary>
    public InkPipeline? Pipeline => pipeline;

    /// <summary>For tests: pin the animation setting instead of following Windows'.</summary>
    internal bool? AssumeReduceMotion
    {
        set
        {
            followsSystem = value is null;
            Perform(schedule.SetReduceMotion(value ?? !SystemMotion.AnimationsEnabled));
        }
    }

    /// <summary>The canvas in pixels for a zone of <paramref name="width"/> x <paramref name="height"/> DIPs at <paramref name="scale"/> (DPI / 96).</summary>
    public static (int Width, int Height) CanvasPixels(double width, double height, double scale)
    {
        var s = Math.Min(scale, width * height > 180_000 ? 1.25 : 2);
        return (Math.Max(2, (int)Math.Round(width * s, MidpointRounding.AwayFromZero)),
            Math.Max(2, (int)Math.Round(height * s, MidpointRounding.AwayFromZero)));
    }

    /// <summary>The host's pixels changed: the canvas is <paramref name="width"/> x <paramref name="height"/>, <paramref name="pointWidth"/> DIPs wide (it sizes the wordmark).</summary>
    public void SetCanvas(int width, int height, double pointWidth)
    {
        if (canvas == (width, height, pointWidth))
        {
            return;
        }
        canvas = (width, height, pointWidth);
        simulation.CanvasWidth = width;
        simulation.CanvasHeight = height;
        DropWordmark();
        UpdateVisibility();
        Perform(schedule.Invalidate());
    }

    /// <summary>The host was shown or hidden.</summary>
    public void SetOnScreen(bool onScreen)
    {
        hostOnScreen = onScreen;
        UpdateVisibility();
    }

    /// <summary>Something else in the host's frame changed (the Drop's text): the frame on screen is out of date.</summary>
    public void Invalidate() => Perform(schedule.Invalidate());

    private void MotionChanged()
    {
        if (followsSystem)
        {
            Perform(schedule.SetReduceMotion(!SystemMotion.AnimationsEnabled));
        }
    }

    /// <summary>The schedule sees a surface that cannot draw (no pipeline, no canvas, failed) as off screen.</summary>
    private void UpdateVisibility() =>
        Perform(schedule.SetOnScreen(hostOnScreen && pipeline is not null && Failure is null && canvas.Width > 0));

    private void Adopt(InkPipelineOutcome outcome)
    {
        if (disposed)
        {
            return;
        }
        pipeline = outcome.Pipeline;
        Failure = outcome.Failure;
        UpdateVisibility();
    }

    private void Perform(InkAction action)
    {
        switch (action)
        {
            case InkAction.DrawStill:
                DrawStill();
                break;
            case InkAction.StartClock:
                firstTick = true;
                clock.Add(this);
                // The first live frame now, not a frame later: a surface just shown must not
                // show what it held when it was last hidden.
                ClockTicked(InkClock.Now());
                break;
            case InkAction.StopClock:
                clock.Remove(this);
                break;
            case InkAction.StopClockAndDrawStill:
                clock.Remove(this);
                DrawStill();
                break;
            case InkAction.Nothing:
            default:
                break;
        }
    }

    /// <summary>One live frame: the prototype's <c>_frame</c>. dt is 1/60 s on the first tick, then the time since the last, clamped to 0..0.05 s.</summary>
    internal void ClockTicked(double now)
    {
        var dt = firstTick ? 0.016 : Math.Min(0.05, Math.Max(0, now - lastTick));
        lastTick = now;
        firstTick = false;
        var live = Levels?.Invoke() ?? InkLevels.Silent;
        simulation.Step(dt, snap: false, InkVoice.Levels(live.Near, live.Far));
        Draw();
    }

    /// <summary>The settled frame: droplets cleared, springs at their targets, no voice.</summary>
    private void DrawStill()
    {
        simulation.Settle(InkVoice.Silent);
        Draw();
    }

    private void Draw()
    {
        if (pipeline is null || canvas.Width <= 0 || disposed)
        {
            return;
        }
        if (ShowsWordmark && wordmark is null && !wordmarkFailed)
        {
            try
            {
                wordmark = Wordmark.Rasterize(pipeline, canvas.Width, canvas.Height, canvas.PointWidth);
            }
            catch (InkRendererException e)
            {
                // A failed wordmark leaves the ink without it; the ink still draws.
                InkLog.Write(e.Message);
                wordmarkFailed = true;
            }
        }
        bool presented;
        try
        {
            presented = target.Render(pipeline, simulation.Uniforms(hasMark: wordmark is not null), wordmark?.Mark);
        }
        catch (InkRendererException e)
        {
            // The device failed (removed, reset, out of memory): stop drawing, say so by name.
            Fail(e.Message);
            return;
        }
        if (presented)
        {
            FramesDrawn++;
            InkFrames.Tick();
        }
    }

    /// <summary>The host's pixels failed (a swapchain that would not resize): stop drawing, say so by name.</summary>
    internal void Fail(string message)
    {
        Failure = message;
        InkLog.Write(message);
        clock.Remove(this);
        UpdateVisibility();
    }

    private void DropWordmark()
    {
        wordmark?.Dispose();
        wordmark = null;
        wordmarkFailed = false;
    }

    /// <summary>Leaves the clock and releases the wordmark. UI thread.</summary>
    public void Dispose()
    {
        if (disposed)
        {
            return;
        }
        disposed = true;
        SystemMotion.Changed -= MotionChanged;
        clock.Remove(this);
        DropWordmark();
    }
}
