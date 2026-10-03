// When an ink surface draws (architecture rule 9: draw nothing when idle), ported from the Mac
// renderer (mac/Sources/InkRenderer/InkSchedule.swift). The clock runs only while a live state, or
// a glide at rest (the main window's orb moving to a new spot, for a couple of seconds), is on
// screen and motion is allowed (Windows' "Animation effects"). Every other change draws at most one
// still frame, and only when the frame on screen is out of date; a hidden surface draws nothing. No
// timer here: each decision follows an event (a glide starting or arriving among them).
namespace Inkwell.Ink;

/// <summary>What a surface does after a change.</summary>
public enum InkAction
{
    /// <summary>Nothing.</summary>
    Nothing,
    /// <summary>Settle the ink and draw one frame.</summary>
    DrawStill,
    /// <summary>Join the clock: each tick steps the ink and draws.</summary>
    StartClock,
    /// <summary>Leave the clock. The last live frame stays on the hidden surface.</summary>
    StopClock,
    /// <summary>Leave the clock, then draw the settled frame.</summary>
    StopClockAndDrawStill,
}

/// <summary>A surface's decisions, kept apart from Win32 and WinUI so they can be tested one by one.</summary>
public struct InkSchedule
{
    /// <summary>What the ink shows.</summary>
    public InkState State { get; private set; }
    /// <summary>Whether the surface is visible.</summary>
    public bool OnScreen { get; private set; }
    /// <summary>Whether animation is off (Windows' Animation effects, off).</summary>
    public bool ReduceMotion { get; private set; }
    /// <summary>Whether the surface is on the clock.</summary>
    public bool ClockRunning { get; private set; }
    /// <summary>The orb is gliding to a new spot at rest.</summary>
    public bool Gliding { get; private set; }
    /// <summary>The frame on screen is the settled frame of the current state at the current size.</summary>
    private bool stillIsCurrent;

    /// <summary>The clock runs only for a live state or a glide, on screen, with motion allowed.</summary>
    public readonly bool WantsClock => (State.IsLive() || Gliding) && OnScreen && !ReduceMotion;

    /// <summary>
    /// The state changed. A change of state also ends a glide: live, the orb wanders on its own
    /// frames; going to rest, it holds where it got to.
    /// </summary>
    public InkAction SetState(InkState state)
    {
        if (state != State)
        {
            State = state;
            stillIsCurrent = false;
            Gliding = false;
        }
        return Update();
    }

    /// <summary>The surface was shown or hidden. Hidden, a glide under way ends (the orb holds where it was).</summary>
    public InkAction SetOnScreen(bool onScreen)
    {
        OnScreen = onScreen;
        if (!onScreen)
        {
            Gliding = false;
        }
        return Update();
    }

    /// <summary>The animation setting changed. Stilled, a glide under way ends.</summary>
    public InkAction SetReduceMotion(bool reduceMotion)
    {
        ReduceMotion = reduceMotion;
        if (reduceMotion)
        {
            Gliding = false;
        }
        return Update();
    }

    /// <summary>A glide at rest starts (true) or arrives (false).</summary>
    public InkAction SetGliding(bool gliding)
    {
        Gliding = gliding;
        return Update();
    }

    /// <summary>The size, the look or the screen changed: the frame on screen is out of date.</summary>
    public InkAction Invalidate()
    {
        stillIsCurrent = false;
        return Update();
    }

    /// <summary>The action for the current inputs.</summary>
    public InkAction Update()
    {
        if (WantsClock)
        {
            stillIsCurrent = false;
            if (ClockRunning)
            {
                return InkAction.Nothing;
            }
            ClockRunning = true;
            return InkAction.StartClock;
        }
        var stopped = ClockRunning;
        if (stopped)
        {
            ClockRunning = false;
            // The surface holds a live frame, not the settled one.
            stillIsCurrent = false;
        }
        if (OnScreen && !stillIsCurrent)
        {
            stillIsCurrent = true;
            return stopped ? InkAction.StopClockAndDrawStill : InkAction.DrawStill;
        }
        return stopped ? InkAction.StopClock : InkAction.Nothing;
    }
}
