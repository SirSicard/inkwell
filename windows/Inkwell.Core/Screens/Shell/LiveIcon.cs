// The icons that show what the app is doing, as the Mac's LiveIcon: on Windows, the taskbar button
// (an overlay badge, its progress, its thumbnail buttons) and the tray icon. Driven by state, never
// by a clock:
//
//   idle        the icon as it is; nothing redraws
//   dictating   the dot in your colour, a still frame
//   recording   the dot in their colour, breathing at 7 fps on a two-second cycle; a still frame
//               in their colour under Always still or with animation effects off
//   final pass  the rim filling with the steps the final pass has reported; indeterminate until
//               the first
//   a problem   the dot in the alert colour
//
// LiveIcon decides what each surface shows and when it redraws. A still look is drawn once, when it
// or its colours change. The recording's pulse is ticked by a timer that runs only while a pulse is
// shown and someone can see the screen; locked or with the display off nothing ticks or draws, and
// on waking the state as it is now is drawn once. Each surface draws the frames it is given. On
// Windows the shell's icons take the still look for every state (LiveIconLook.OnShell): there,
// every frame is a call into Explorer.
using Inkwell.Core.Glow;

namespace Inkwell.Core.Screens;

/// <summary>Whose colour a look takes.</summary>
public enum LiveIconTone
{
    You,
    Them,
    Alert,
}

/// <summary>What the icon shows.</summary>
public abstract record LiveIconLook
{
    private LiveIconLook() { }

    /// <summary>The icon as it is.</summary>
    public sealed record Rest : LiveIconLook;

    /// <summary>The dot in a colour, still.</summary>
    public sealed record Glow(LiveIconTone Tone) : LiveIconLook;

    /// <summary>The dot in a colour, breathing.</summary>
    public sealed record Pulse(LiveIconTone Tone) : LiveIconLook;

    /// <summary>The rim filled to the final pass's progress (0..1), or indeterminate.</summary>
    public sealed record Ring(double? Progress) : LiveIconLook;

    /// <summary>
    /// The look for <paramref name="state"/>. <paramref name="progress"/>: the final pass's, when
    /// there is a number. <paramref name="still"/>: Always still or animations off, which hold the
    /// recording still in its colour.
    /// </summary>
    public static LiveIconLook For(DropInk state, double? progress, bool still) => state switch
    {
        DropInk.Dictating => new Glow(LiveIconTone.You),
        DropInk.Meeting => still ? new Glow(LiveIconTone.Them) : new Pulse(LiveIconTone.Them),
        DropInk.Blotting => new Ring(progress),
        DropInk.Problem => new Glow(LiveIconTone.Alert),
        _ => new Rest(),
    };

    /// <summary>
    /// The look for <paramref name="state"/> on Windows' shell icons, the tray icon and the taskbar
    /// button's badge (LiveIconHost): always still, a recording in their colour. Each frame there is
    /// a call into Explorer on the UI thread (Shell_NotifyIcon, ITaskbarList3.SetOverlayIcon), seven
    /// a second for a breath, and a hung Explorer would stall the app with it; held still, they
    /// change only with the state, and no timer runs. The window's own ink still moves.
    /// </summary>
    public static LiveIconLook OnShell(DropInk state, double? progress) => For(state, progress, still: true);

    /// <summary>Whether it moves (the only look that runs a timer).</summary>
    public bool Pulses => this is Pulse;
}

/// <summary>The colours the icons draw in: the night mode's (the icon's plate is night in either mode).</summary>
public sealed record LiveIconColours(GlowRgb You, GlowRgb Them, GlowRgb Alert)
{
    public static LiveIconColours Unset { get; } = new(default, default, default);

    /// <summary>The colour of <paramref name="tone"/>.</summary>
    public GlowRgb Colour(LiveIconTone tone) => tone switch
    {
        LiveIconTone.You => You,
        LiveIconTone.Them => Them,
        _ => Alert,
    };
}

/// <summary>One frame of the icon.</summary>
/// <param name="Strength">How strongly the colour shows, 0..1: 1 for a still look; the pulse breathes it.</param>
public sealed record LiveIconFrame(LiveIconLook Look, LiveIconColours Colours, double Strength);

/// <summary>Something that shows the icon: the taskbar button, the tray icon.</summary>
public interface ILiveIconSurface
{
    void Show(LiveIconFrame frame);
}

/// <summary>The pulse's clock.</summary>
public interface ILiveIconTicker
{
    bool Running { get; }

    /// <summary>Calls <paramref name="tick"/> every <paramref name="interval"/>, on the UI thread, until stopped.</summary>
    void Start(TimeSpan interval, Action tick);

    void Cancel();
}

/// <summary>The icons' state and their redraws. UI thread.</summary>
public sealed class LiveIcon(ILiveIconTicker ticker)
{
    // The pulse (Pulse, Breath, the ticker) is the Mac's LiveIcon, ported with its tests: kept so
    // the looks stay one decision (For), though Windows' shell never shows it (OnShell).

    /// <summary>The pulse's frame rate: a breath reads as smooth at 7 fps, at a fraction of a display's rate.</summary>
    public const double PulseFps = 7;

    /// <summary>One breath, out and in, in seconds.</summary>
    public const double BreathPeriod = 2;

    /// <summary>The faintest the pulse gets: it dims, never goes out, so the recording always shows.</summary>
    public const double BreathLow = 0.55;

    private readonly List<ILiveIconSurface> surfaces = [];
    /// <summary>Ticks into the current breath.</summary>
    private int tick;
    /// <summary>A frame came while nobody could see it: drawn on waking.</summary>
    private bool unshown;

    public LiveIconFrame Frame { get; private set; } = new(new LiveIconLook.Rest(), LiveIconColours.Unset, 1);

    /// <summary>
    /// Frames drawn since launch: each change, and each tick of the pulse. The energy budget's count
    /// (0 a minute at rest, 1 a change while dictating, 420 a minute while recording).
    /// </summary>
    public int Frames { get; private set; }

    /// <summary>Someone can see the screen.</summary>
    public bool IsAwake { get; private set; } = true;

    public bool IsPulsing => ticker.Running;

    /// <summary>Shows <paramref name="surface"/> the current frame (unless nobody can see it), and every frame from now on.</summary>
    public void Attach(ILiveIconSurface surface)
    {
        ArgumentNullException.ThrowIfNull(surface);
        if (surfaces.Contains(surface))
        {
            return;
        }
        surfaces.Add(surface);
        if (IsAwake)
        {
            surface.Show(Frame);
        }
        else if (Frame.Look is not LiveIconLook.Rest)
        {
            unshown = true;
        }
        Reconcile();
    }

    public void Detach(ILiveIconSurface surface)
    {
        surfaces.Remove(surface);
        Reconcile();
    }

    /// <summary>
    /// Whether anyone can see the screen. Asleep, nothing ticks or draws (a change is kept); awake
    /// again, the frame as it is now is drawn if it changed, and the breath resumes.
    /// </summary>
    public void SetAwake(bool awake)
    {
        if (awake == IsAwake)
        {
            return;
        }
        IsAwake = awake;
        if (awake && unshown)
        {
            Deliver(Frame);
        }
        Reconcile();
    }

    /// <summary>The look and colours now. The same again draws nothing; a pulse already running keeps its place in the breath.</summary>
    public void Update(LiveIconLook look, LiveIconColours colours)
    {
        ArgumentNullException.ThrowIfNull(look);
        ArgumentNullException.ThrowIfNull(colours);
        // At rest the icon shows no colour: a theme change (or the first colours at launch) is
        // kept for later and draws nothing.
        if (look is LiveIconLook.Rest && Frame.Look is LiveIconLook.Rest)
        {
            Frame = Frame with { Colours = colours };
            return;
        }
        var next = Frame with { Look = look, Colours = colours };
        if (look != Frame.Look)
        {
            tick = 0;
            next = next with { Strength = 1 };
        }
        if (next == Frame)
        {
            return;
        }
        Deliver(next);
        Reconcile();
    }

    /// <summary>
    /// The final pass's progress, 0..1, from the steps the core has reported (the ones Today lists):
    /// each side transcribed, the far end's speakers told apart, the summary written. A step the
    /// core skips is never reported, so the ring may not reach full before the pass ends; it never
    /// claims a step that has not happened. Null (indeterminate) before the first.
    /// </summary>
    public static double? FinalPassProgress(LiveMeeting? meeting)
    {
        if (meeting is null)
        {
            return null;
        }
        var b = meeting.Blotted;
        // A later step comes after both sides.
        var later = b.Diarized || b.Summarized;
        var sides = later ? 2 : Math.Min(b.SidesTranscribed, 2);
        var steps = sides + (b.Diarized ? 1 : 0) + (b.Summarized ? 1 : 0);
        return steps == 0 ? null : steps / 4.0;
    }

    /// <summary>The strength <paramref name="tick"/> frames into a breath: full, out to BreathLow, and back.</summary>
    public static double Breath(int tick)
    {
        var phase = tick / (PulseFps * BreathPeriod);
        return BreathLow + (1 - BreathLow) * (0.5 + 0.5 * Math.Cos(2 * Math.PI * phase));
    }

    /// <summary>The taskbar thumbnail toolbar's button, while nothing records and while something does.</summary>
    public const string RecordButton = "Record";
    public const string StopButton = "Stop";

    /// <summary>What Narrator hears for the tray icon in <paramref name="state"/> (the Mac's menu-bar label).</summary>
    public static string Spoken(DropInk state) => state switch
    {
        DropInk.Dictating => "Inkwell, dictating",
        DropInk.Meeting => "Inkwell, recording",
        DropInk.Blotting => "Inkwell, finishing a recording",
        DropInk.Problem => "Inkwell, recording; the other side is quiet",
        _ => "Inkwell",
    };

    /// <summary>The taskbar overlay's description (Narrator reads it with the button's name), or null at rest.</summary>
    public static string? OverlayText(DropInk state) => state switch
    {
        DropInk.Dictating => "Dictating",
        DropInk.Meeting => "Recording",
        DropInk.Blotting => "Finishing a recording",
        DropInk.Problem => "Recording; the other side is quiet",
        _ => null,
    };

    private void Deliver(LiveIconFrame next)
    {
        Frame = next;
        if (!IsAwake)
        {
            unshown = true;
            return;
        }
        unshown = false;
        Frames++;
        foreach (var surface in surfaces.ToList())
        {
            surface.Show(next);
        }
    }

    /// <summary>Runs the timer exactly while a pulse is shown on a surface and someone can see it.</summary>
    private void Reconcile()
    {
        var wanted = IsAwake && Frame.Look.Pulses && surfaces.Count > 0;
        if (wanted && !ticker.Running)
        {
            ticker.Start(TimeSpan.FromSeconds(1 / PulseFps), Advance);
        }
        else if (!wanted && ticker.Running)
        {
            ticker.Cancel();
        }
    }

    private void Advance()
    {
        // A tick that lands after the timer was stopped (asleep, or the pulse over) draws nothing.
        if (!IsAwake || !Frame.Look.Pulses || surfaces.Count == 0)
        {
            return;
        }
        tick = (tick + 1) % (int)(PulseFps * BreathPeriod);
        Frame = Frame with { Strength = Breath(tick) };
        Frames++;
        foreach (var surface in surfaces.ToList())
        {
            surface.Show(Frame);
        }
    }
}
