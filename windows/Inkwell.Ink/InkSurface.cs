// One ink on screen: what the Mac's InkView does, apart from AppKit. A host (the Drop's window, a
// SwapChainPanel) owns the pixels and says when it is visible and how big it is; the surface keeps
// the state, the simulation and the schedule, and asks the host to present a frame only when the
// schedule says so: on the shared clock while live, one still frame otherwise, nothing while
// hidden. Every frame presented is counted (InkFrames). UI thread only.
//
// The canvas follows the prototype's sizing rule, as on the Mac: the display scale, capped at 1.25
// for a large canvas (over 180,000 square DIPs) and at 2 otherwise.
//
// A frame that fails is not the end: the surface drops the host's device objects and tries again
// after 0.5, 1 and 2 s, then every 5 s, for as long as the host is on screen (hidden, it waits for
// the next show: nothing ticks for a hidden ink). Only a lost device (TDR, driver update: the
// device reports itself removed, or the HRESULT says so) costs a new pipeline; any other failure (a
// composition device lost to a DWM restart, a swapchain that cannot be made) keeps the pipeline,
// and a retry only makes the host's objects again. A failure is announced once, not per retry.
//
// A host that must never go blank (the Drop, WatchesDevice) is also checked while it shows and
// draws nothing (Animation effects off, or a still frame): once a second, the device's removed
// reason and the host's own objects (IInkTarget.CheckDevice). A lost device or a DWM restart is
// then noticed without a frame. The check runs only while such a host is on screen: never idle. While it cannot draw (the pipeline
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
    bool Render(InkPipeline pipeline, in InkUniforms uniforms);

    /// <summary>The pipeline is lost or replaced: release every object made on its device.</summary>
    void ReleaseDeviceResources();

    /// <summary>
    /// Whether to show what the host shows when the ink cannot draw (the Drop: a plain panel with
    /// its text). Called on every change.
    /// </summary>
    void SetFallback(bool shown);

    /// <summary>
    /// Why the host's own device objects no longer reach the screen (a composition device lost to a
    /// DWM restart), or null when they are fine. Called about once a second while a watched host
    /// shows and draws nothing.
    /// </summary>
    string? CheckDevice();
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
    /// <summary>The host's objects failed on a device that is still good: the next retry makes them again.</summary>
    private bool hostBroken;
    /// <summary>Recovery attempts since the last frame that drew.</summary>
    private int attempts;
    private bool retryScheduled;
    private bool redrawScheduled;
    private Timer? redrawTimer;
    private bool retryWhenShown;
    private Timer? retryTimer;
    private bool fallbackShown;
    private bool healthScheduled;
    private Timer? healthTimer;
    private readonly InkSimulation simulation = new();
    private InkSchedule schedule;
    private InkPipeline? pipeline;
    private bool hostOnScreen;
    private bool followsSystem = true;
    private bool disposed;
    private double lastTick;
    private bool firstTick = true;
    private (int Width, int Height) canvas;

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
        schedule.SetReduceMotion(SystemReducesMotion);
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

    /// <summary>Where the orb sits on the canvas: the window's place, or the Drop's.</summary>
    public InkPlacement Placement
    {
        get;
        set
        {
            if (field == value)
            {
                return;
            }
            field = value;
            Perform(schedule.Invalidate());
        }
    } = InkPlacement.Centre;

    /// <summary>The orb's colours (the shell's appearance).</summary>
    public GlowLook Look
    {
        get;
        set
        {
            ArgumentNullException.ThrowIfNull(value);
            if (field == value)
            {
                return;
            }
            field = value;
            Perform(schedule.Invalidate());
        }
    } = GlowLook.Default;

    /// <summary>
    /// The user's "Always still" (Settings > Appearance): one still frame per change, whatever
    /// Windows' Animation effects say.
    /// </summary>
    public bool AlwaysStill
    {
        get;
        set
        {
            if (field == value)
            {
                return;
            }
            field = value;
            if (followsSystem)
            {
                Perform(schedule.SetReduceMotion(SystemReducesMotion));
            }
        }
    }

    /// <summary>The prototype's stand-in voice instead of the live levels (the first run's demo).</summary>
    public bool Demo { get; set; }

    /// <summary>How far the final pass's blot goes (InkSimulation.BlotDepth): 1, the Drop's, unless set.</summary>
    public double BlotDepth
    {
        get => simulation.BlotDepth;
        set
        {
            simulation.BlotDepth = value;
            Perform(schedule.Invalidate());
        }
    }

    /// <summary>
    /// Each frame's state, as it is drawn (live on the clock, or the still frame): the window's edge
    /// glow follows it, so it moves exactly when the orb does and never on its own. UI thread.
    /// </summary>
    public event Action<GlowFrame>? Drawn;

    /// <summary>Frames this surface has presented.</summary>
    public int FramesDrawn { get; private set; }

    /// <summary>Whether the surface is on the clock (live, on screen, motion allowed).</summary>
    public bool IsAnimating => clock.Contains(this);

    /// <summary>Whether the pipeline has arrived. Until then the host shows its fallback and no clock runs.</summary>
    public bool IsReady => pipeline is not null;

    /// <summary>Why the ink cannot draw, if it cannot.</summary>
    public string? Failure { get; private set; }

    /// <summary>The canvas in pixels.</summary>
    public (int Width, int Height) Canvas => canvas;

    /// <summary>The pipeline, once it has arrived (hosts make their swapchains on its device).</summary>
    public InkPipeline? Pipeline => pipeline;

    /// <summary>A failure began (its message) or ended (null). UI thread.</summary>
    public event Action<string?>? FailureChanged;

    /// <summary>How a retry waits (tests replace it): by default a one-shot timer, then the UI thread.</summary>
    internal Action<TimeSpan, Action>? Scheduler { get; set; }

    /// <summary>How a device check waits (tests replace it): by default a one-shot timer, then the UI thread.</summary>
    internal Action<TimeSpan, Action>? HealthScheduler { get; set; }

    /// <summary>How often a watched host's device is checked while it shows and draws nothing.</summary>
    internal static readonly TimeSpan HealthInterval = TimeSpan.FromSeconds(1);

    /// <summary>
    /// Check the device about once a second while the host shows and nothing draws, so a lost
    /// device or DWM restart is noticed without a frame (the Drop, which must never go blank).
    /// </summary>
    public bool WatchesDevice
    {
        get;
        set
        {
            field = value;
            UpdateHealth();
        }
    }

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
            Perform(schedule.SetReduceMotion(value ?? SystemReducesMotion));
        }
    }

    /// <summary>The canvas in pixels for a zone of <paramref name="width"/> x <paramref name="height"/> DIPs at <paramref name="scale"/> (DPI / 96).</summary>
    public static (int Width, int Height) CanvasPixels(double width, double height, double scale)
    {
        var s = Math.Min(scale, width * height > 180_000 ? 1.25 : 2);
        return (Math.Max(2, (int)Math.Round(width * s, MidpointRounding.AwayFromZero)),
            Math.Max(2, (int)Math.Round(height * s, MidpointRounding.AwayFromZero)));
    }

    /// <summary>The host's pixels changed: the canvas is <paramref name="width"/> x <paramref name="height"/>.</summary>
    public void SetCanvas(int width, int height)
    {
        if (canvas == (width, height))
        {
            return;
        }
        canvas = (width, height);
        simulation.CanvasWidth = width;
        simulation.CanvasHeight = height;
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

    /// <summary>Whether the ink holds still: Animation effects off, or the user's Always still.</summary>
    private bool SystemReducesMotion => AlwaysStill || !SystemMotion.AnimationsEnabled;

    private void MotionChanged()
    {
        if (followsSystem)
        {
            Perform(schedule.SetReduceMotion(SystemReducesMotion));
        }
    }

    /// <summary>
    /// The schedule sees a surface that cannot draw (no pipeline, no canvas) as off screen; the
    /// host shows its fallback while it is on screen and the ink cannot draw or is failing (it
    /// stays up through a retry until a frame really draws, so it never blinks).
    /// </summary>
    private void UpdateVisibility()
    {
        UpdateFallback();
        Perform(schedule.SetOnScreen(hostOnScreen && CanDraw));
        UpdateHealth();
    }

    private bool HealthWanted => WatchesDevice && hostOnScreen && CanDraw && !schedule.ClockRunning && !disposed;

    /// <summary>Starts the next device check when one is wanted and none is waiting.</summary>
    private void UpdateHealth()
    {
        if (!HealthWanted || healthScheduled)
        {
            return;
        }
        healthScheduled = true;
        if (HealthScheduler is { } scheduler)
        {
            scheduler(HealthInterval, HealthTick);
            return;
        }
        healthTimer?.Dispose();
        healthTimer = new Timer(_ => clock.Post(HealthTick), null, HealthInterval, Timeout.InfiniteTimeSpan);
    }

    private void HealthTick()
    {
        healthScheduled = false;
        healthTimer?.Dispose();
        healthTimer = null;
        if (!HealthWanted)
        {
            // Hidden, live on the clock (frames find failures), or already failing: no check.
            return;
        }
        if (pipeline!.DeviceRemoved())
        {
            DeviceFailed("couldn't keep the Direct3D device (it was removed or reset)", deviceLost: true);
            return;
        }
        string? problem;
        try
        {
            problem = target.CheckDevice();
        }
        catch (Exception e)
        {
            problem = $"couldn't check the ink's device: {e.GetType().Name}: {e.Message}";
        }
        if (problem is not null)
        {
            DeviceFailed(problem);
            return;
        }
        UpdateHealth();
    }

    private bool CanDraw => pipeline is not null && canvas.Width > 0 && !hostBroken;

    private void UpdateFallback()
    {
        var fallback = hostOnScreen && (!CanDraw || Failure is not null) && !disposed;
        if (fallback != fallbackShown)
        {
            fallbackShown = fallback;
            target.SetFallback(fallback);
        }
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
            hostBroken = false;
            // The frame on screen is from the old device (or none): draw anew once on screen.
            Perform(schedule.Invalidate());
        }
        UpdateVisibility();
    }

    /// <summary>
    /// A frame failed: drop the host's objects, show the fallback, and try again with backoff. A lost
    /// device (<paramref name="deviceLost"/>, or the pipeline's device says so) also drops the
    /// pipeline, and the retry asks for a new one.
    /// </summary>
    internal void DeviceFailed(string message, bool deviceLost = false)
    {
        SetFailure(message);
        clock.Remove(this);
        target.ReleaseDeviceResources();
        if (deviceLost || pipeline?.DeviceRemoved() == true)
        {
            lost = pipeline;
            pipeline = null;
            hostBroken = false;
        }
        else
        {
            hostBroken = true;
        }
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

    /// <summary>About a frame: how long a dropped still frame waits to be drawn again.</summary>
    internal static readonly TimeSpan RedrawDelay = TimeSpan.FromMilliseconds(16);

    /// <summary>
    /// The host's present was dropped: the compositor had no room for it
    /// (CompositionSwapChain.Present). On the clock nothing is needed, the next tick draws; a still
    /// frame (a fade's end, a theme, a change with motion off) is drawn again a frame later, else
    /// it would stay undrawn until something else changed. UI thread.
    /// </summary>
    public void PresentDropped()
    {
        if (redrawScheduled || disposed || IsAnimating)
        {
            return;
        }
        redrawScheduled = true;
        if (Scheduler is { } scheduler)
        {
            scheduler(RedrawDelay, Redraw);
            return;
        }
        redrawTimer?.Dispose();
        redrawTimer = new Timer(_ => clock.Post(Redraw), null, RedrawDelay, Timeout.InfiniteTimeSpan);
    }

    private void Redraw()
    {
        redrawScheduled = false;
        redrawTimer?.Dispose();
        redrawTimer = null;
        if (!disposed)
        {
            Invalidate();
        }
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
        if (disposed)
        {
            return;
        }
        if (!hostOnScreen)
        {
            retryWhenShown = true;
            return;
        }
        if (pipeline is not null)
        {
            if (hostBroken)
            {
                // The device is good: the host makes its objects again on the next frame.
                hostBroken = false;
                Perform(schedule.Invalidate());
                UpdateVisibility();
            }
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
        UpdateHealth();
    }

    /// <summary>One live frame: the prototype's <c>_frame</c>. dt is 1/60 s on the first tick, then the time since the last, clamped to 0..0.05 s.</summary>
    internal void ClockTicked(double now)
    {
        var dt = firstTick ? 0.016 : Math.Min(0.05, Math.Max(0, now - lastTick));
        lastTick = now;
        firstTick = false;
        var live = Levels?.Invoke() ?? InkLevels.Silent;
        simulation.Step(dt, snap: false, Demo ? InkVoice.Synthetic : InkVoice.Levels(live.Near, live.Far));
        Drawn?.Invoke(simulation.Frame(moving: true));
        Draw(moving: true);
    }

    /// <summary>The ink at rest in its state, drawing nothing (a host that hid: its next frame starts settled).</summary>
    public void Settle() => simulation.Settle(InkVoice.Silent);

    /// <summary>The settled frame: droplets cleared, springs at their targets, no voice.</summary>
    private void DrawStill()
    {
        simulation.Settle(InkVoice.Silent);
        Drawn?.Invoke(simulation.Frame(moving: false));
        Draw(moving: false);
    }


    /// <summary>One frame: live (<paramref name="moving"/>) on the clock, or the still frame, whose time stands still.</summary>
    private void Draw(bool moving)
    {
        if (pipeline is null || canvas.Width <= 0 || disposed)
        {
            return;
        }
        bool presented;
        try
        {
            presented = target.Render(pipeline, simulation.Uniforms(Placement, Look, moving));
        }
        catch (InkRendererException e)
        {
            // The device failed (removed, reset, DWM restarted, out of memory): recover.
            DeviceFailed(e.Message, InkPipeline.IsDeviceLoss(e.HResultCode));
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
                UpdateFallback();
            }
        }
    }

    /// <summary>Leaves the clock. UI thread.</summary>
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
        redrawTimer?.Dispose();
        redrawTimer = null;
        healthTimer?.Dispose();
        healthTimer = null;
        SystemMotion.Changed -= MotionChanged;
        clock.Remove(this);
    }
}
