// One ink on screen: what the Mac's InkView does, apart from AppKit. A host (the Drop's window, a
// SwapChainPanel) owns the pixels and says when it is visible and how big it is; the surface keeps
// the state, the simulation and the schedule, and asks the host to present a frame only when the
// schedule says so: on the shared clock while live, one still frame otherwise, nothing while
// hidden. Every frame presented is counted (InkFrames). UI thread only.
//
// The canvas follows the prototype's sizing rule, as on the Mac: the display scale, capped at 1.25
// for a large canvas (over 180,000 square DIPs) and at 2 otherwise.
//
// A frame that fails (a lost device: TDR, driver update, DWM restart; a failed present) is not the
// end: the surface drops the host's device objects, asks the loader for a new pipeline, and tries
// again after 0.5, 1 and 2 s, then every 5 s, for as long as the host is on screen (hidden, it
// waits for the next show: nothing ticks for a hidden ink). While it cannot draw (the pipeline
// still compiling, lost, or a shader that does not compile) the host shows its fallback, and every
// change of failure is announced (FailureChanged) so the shell can show it.
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

    /// <summary>The pipeline is lost or replaced: release every object made on its device.</summary>
    void ReleaseDeviceResources();

    /// <summary>
    /// Whether to show what the host shows when the ink cannot draw (the Drop: a plain panel with
    /// its text). Called on every change.
    /// </summary>
    void SetFallback(bool shown);
}

/// <summary>One ink on screen, drawn into a host.</summary>
public sealed class InkSurface : IDisposable
{
    private readonly IInkTarget target;
    private readonly InkClock clock;
    private readonly InkPipelineLoader loader;
    private readonly IDisposable subscription;
    /// <summary>The pipeline whose frame failed, until a new one arrives.</summary>
    private InkPipeline? lost;
    /// <summary>Recovery attempts since the last frame that drew.</summary>
    private int attempts;
    private bool retryScheduled;
    private bool retryWhenShown;
    private Timer? retryTimer;
    private bool fallbackShown;
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
        this.loader = loader;
        // The prototype starts each ink at a random point in its slow motion.
        simulation.T = Random.Shared.NextDouble() * 30;
        schedule.SetReduceMotion(!SystemMotion.AnimationsEnabled);
        SystemMotion.Changed += MotionChanged;
        // A pipeline already made is used at once; this and every later outcome also arrive
        // through the subscription (Adopt ignores the one it already has).
        if (loader.Outcome is { } known)
        {
            Adopt(known);
        }
        subscription = loader.Subscribe(clock.Post, Adopt);
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

    /// <summary>A failure began (its message) or ended (null). UI thread.</summary>
    public event Action<string?>? FailureChanged;

    /// <summary>How a retry waits (tests replace it): by default a one-shot timer, then the UI thread.</summary>
    internal Action<TimeSpan, Action>? Scheduler { get; set; }

    /// <summary>The waits between recovery attempts: 0.5, 1 and 2 s, then every 5 s.</summary>
    internal static TimeSpan RetryDelay(int attempt) => TimeSpan.FromSeconds(attempt switch
    {
        1 => 0.5,
        2 => 1,
        3 => 2,
        _ => 5,
    });

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
        if (onScreen && retryWhenShown)
        {
            retryWhenShown = false;
            ScheduleRetry();
        }
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

    /// <summary>
    /// The schedule sees a surface that cannot draw (no pipeline, no canvas) as off screen; the
    /// host shows its fallback while it is on screen and the ink cannot draw.
    /// </summary>
    private void UpdateVisibility()
    {
        var canDraw = pipeline is not null && canvas.Width > 0;
        var fallback = hostOnScreen && !canDraw && !disposed;
        if (fallback != fallbackShown)
        {
            fallbackShown = fallback;
            target.SetFallback(fallback);
        }
        Perform(schedule.SetOnScreen(hostOnScreen && canDraw));
    }

    private void Adopt(InkPipelineOutcome outcome)
    {
        if (disposed || (outcome.Pipeline is not null && outcome.Pipeline == pipeline))
        {
            return;
        }
        if (outcome.Pipeline is null && outcome.Failure is null)
        {
            return;
        }
        if (pipeline is not null)
        {
            // Replaced after another surface lost the device: this one's objects are on it too.
            target.ReleaseDeviceResources();
            DropWordmark();
            pipeline = null;
            // Off the clock through the schedule, so it knows; no fallback flashes in between.
            Perform(schedule.SetOnScreen(false));
        }
        pipeline = outcome.Pipeline;
        if (pipeline is null)
        {
            SetFailure(outcome.Failure);
            if (!outcome.Permanent)
            {
                ScheduleRetry();
            }
        }
        else
        {
            lost = null;
            // The frame on screen is from the old device (or none): draw anew once on screen.
            Perform(schedule.Invalidate());
        }
        UpdateVisibility();
    }

    /// <summary>A frame failed: drop the device's objects, show the fallback, and try again with backoff.</summary>
    internal void DeviceFailed(string message)
    {
        SetFailure(message);
        clock.Remove(this);
        target.ReleaseDeviceResources();
        DropWordmark();
        lost = pipeline;
        pipeline = null;
        UpdateVisibility();
        ScheduleRetry();
    }

    private void SetFailure(string? message)
    {
        if (message == Failure)
        {
            return;
        }
        Failure = message;
        InkLog.Write(message ?? "the ink draws again");
        FailureChanged?.Invoke(message);
    }

    private void ScheduleRetry()
    {
        if (retryScheduled || disposed)
        {
            return;
        }
        if (!hostOnScreen)
        {
            retryWhenShown = true;
            return;
        }
        attempts++;
        retryScheduled = true;
        var delay = RetryDelay(attempts);
        if (Scheduler is { } scheduler)
        {
            scheduler(delay, Retry);
            return;
        }
        retryTimer?.Dispose();
        retryTimer = new Timer(_ => clock.Post(Retry), null, delay, Timeout.InfiniteTimeSpan);
    }

    private void Retry()
    {
        retryScheduled = false;
        retryTimer?.Dispose();
        retryTimer = null;
        if (disposed || pipeline is not null)
        {
            return;
        }
        if (!hostOnScreen)
        {
            retryWhenShown = true;
            return;
        }
        if (loader.Outcome is { Pipeline: { } current } && current != lost)
        {
            // Another surface already has a new one.
            Adopt(loader.Outcome);
            return;
        }
        if (loader.Outcome is { Permanent: true })
        {
            return;
        }
        // The new outcome arrives through the subscription; a failed one schedules the next try.
        loader.Recreate(lost, clock.Post);
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
            // The device failed (removed, reset, DWM restarted, out of memory): recover.
            DeviceFailed(e.Message);
            return;
        }
        catch (Exception e)
        {
            // Anything else from a host's frame is a failure too: named, the fallback shown, and
            // retried the same way. It never escapes into a clock tick or a window procedure.
            DeviceFailed($"couldn't draw the ink: {e.GetType().Name}: {e.Message}");
            return;
        }
        if (presented)
        {
            FramesDrawn++;
            InkFrames.Tick();
            if (Failure is not null)
            {
                attempts = 0;
                SetFailure(null);
            }
        }
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
        subscription.Dispose();
        retryTimer?.Dispose();
        retryTimer = null;
        SystemMotion.Changed -= MotionChanged;
        clock.Remove(this);
        DropWordmark();
    }
}
