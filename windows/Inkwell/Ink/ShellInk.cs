// The ink the shell shows: one state for every surface that draws it (the Drop, an InkPanel in the
// window), the process's pipeline and frame clock, and the Drop itself. The Mac's ShellInk
// (mac/Sources/Inkwell/ShellInk.swift). What the core says arrives through Show, from the app's
// DropModel (Inkwell.Core): a meeting's line with the ink recording, blotting or in trouble, a
// take's line with the ink dictating, a note after a take or the offer to record a call with the
// ink still, or nothing (the Drop hides). The offer's buttons go to the app's callback as the
// model's actions (DropAction). Held pins a state for the Drop's focus check
// (INK_DROP_DEMO). Every ink reads the live levels (your mic's bands, and the far end's during a
// meeting) once per frame while it moves. The Drop follows the app's appearance (SetLook): its pill
// in the mode, its orb in the user's colours, still when the user wants it still. Nothing here polls.
//
// When the Drop cannot draw its ink (a lost device it is recovering from, a shader that does not
// compile, no plain panel either, no window at all) the problem goes to the callback the app
// gives (the window's status line and the tray icon, which stays seen while the window is hidden)
// as well as to the log; null says it is fine again. A Drop window that could not be made is
// tried again at every change of state.
using Inkwell.Core;
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Inkwell.Ink;
using Microsoft.UI.Dispatching;

namespace Inkwell;

internal sealed class ShellInk : IDisposable
{
    private DropWindow? drop;
    private readonly Action<string?> report;
    private readonly Action<DropAction> act;
    /// <summary>The line shown, whose buttons a click answers.</summary>
    private DropLine? shown;
    private readonly DropDemo? demo;
    private InkState live = InkState.Idle;
    private DropText? text;
    private InkState? held;
    /// <summary>Quitting: a note's end or a last batch arriving after it changes nothing (no new Drop is made).</summary>
    private bool disposed;
    private GlowLook orbLook = GlowLook.Default;
    private DropLook pillLook = DropLook.Default;
    private bool alwaysStill;

    /// <summary>The frame clock every ink in the process shares.</summary>
    public InkClock Clock { get; }

    /// <summary>The pipeline, compiled once off the UI thread from launch.</summary>
    public static InkPipelineLoader Loader => InkPipelineLoader.Shared;

    /// <summary>
    /// UI thread. Starts the pipeline's compile and makes the (hidden) Drop; <paramref name="showFailure"/>
    /// shows why the ink cannot draw (null: it draws); <paramref name="act"/> runs a button the Drop offers.
    /// </summary>
    public ShellInk(DispatcherQueue ui, Action<string?> showFailure, Action<DropAction> act)
    {
        this.act = act;
        Clock = new InkClock(work =>
        {
            // False only while the app shuts down, when no frame matters any more.
            if (!ui.TryEnqueue(() => work()))
            {
                InkLog.Write("couldn't reach the UI thread for a frame (the app is quitting)");
            }
        });
        Loader.Warm();
        Loader.WhenReady(Clock.Post, outcome =>
        {
            if (outcome.Pipeline is { } p)
            {
                InkLog.Write($"drawing on {p.AdapterName}{(p.IsWarp ? " (WARP, the software rasteriser)" : "")}, shader compiled in {p.CompileTime.TotalMilliseconds:F0} ms");
            }
        });
        report = showFailure;
        MakeDrop();
        if (DropDemo.Interval(Environment.GetEnvironmentVariable("INK_DROP_DEMO")) is { } interval)
        {
            demo = new DropDemo(ui, this, interval);
        }
    }

    /// <summary>What the ink shows now.</summary>
    public InkState State => held ?? live;

    /// <summary>The state changed (the window's inks follow it).</summary>
    public event Action? Changed;

    /// <summary>
    /// UI thread. What the Drop says (<paramref name="line"/>; null hides it, its actions become its
    /// buttons) and what its ink shows (<paramref name="ink"/>: still for a note or an offer).
    /// </summary>
    public void Show(DropLine? line, DropInk ink)
    {
        shown = line;
        text = line is null
            ? null
            : new DropText(line.Title, line.Detail, Tone(line.Tone), line.LiveWords)
            {
                Buttons = line.Actions is { } a ? new DropButtons(a.All.Select(action => action.Title).ToList()) : null,
                DetailLines = line.DetailLines,
            };
        live = InkFor(ink);
        Update();
    }

    /// <summary>The renderer's state for the model's ink.</summary>
    internal static InkState InkFor(DropInk ink) => ink switch
    {
        DropInk.Dictating => InkState.Dictating,
        DropInk.Meeting => InkState.Meeting,
        DropInk.Blotting => InkState.Blotting,
        DropInk.Problem => InkState.Problem,
        _ => InkState.Idle,
    };

    /// <summary>A button of the Drop was clicked: its action, if the line shown still offers it.</summary>
    private void Clicked(int index)
    {
        if (held is null && shown?.Actions?.At(index) is { } action)
        {
            act(action);
        }
    }

    private static DropTone Tone(DropLineTone tone) => tone switch
    {
        DropLineTone.Recording => DropTone.Recording,
        DropLineTone.Alert => DropTone.Alert,
        _ => DropTone.Plain,
    };

    /// <summary>
    /// The live levels, read by each ink once per frame while it moves: your mic's bands and the
    /// far end's, each on the Mac's decibel map. ink_bands_read never locks or allocates.
    /// </summary>
    public static InkLevels LiveLevels()
    {
        var near = InkSession.Bands();
        var far = InkSession.FarBands();
        return new InkLevels(InkLevels.Level(near.Low, near.Mid, near.High), InkLevels.Level(far.Low, far.Mid, far.High));
    }

    /// <summary>UI thread. The app's appearance: the Drop's pill and orb follow it.</summary>
    public void SetLook(GlowLook orb, DropLook pill, bool still)
    {
        orbLook = orb;
        pillLook = pill;
        alwaysStill = still;
        if (drop is not null)
        {
            ApplyLook(drop);
        }
    }

    private void ApplyLook(DropWindow window)
    {
        window.Look = pillLook;
        window.Surface.Look = orbLook;
        window.Surface.AlwaysStill = alwaysStill;
    }

    /// <summary>A state held whatever the core says (the Drop's focus check). Null in ordinary use.</summary>
    public InkState? Held
    {
        get => held;
        set
        {
            held = value;
            Update();
        }
    }

    /// <summary>Makes the Drop's window if it is not there; a failure is said, and tried again at the next change.</summary>
    private void MakeDrop()
    {
        if (drop is not null)
        {
            return;
        }
        try
        {
            drop = new DropWindow(Loader, Clock);
            drop.Surface.Levels = LiveLevels;
            drop.Surface.Placement = new InkPlacement(GlowTokens.Orb.Drop.X, GlowTokens.Orb.Drop.Y, GlowTokens.Orb.Drop.Unit);
            ApplyLook(drop);
            drop.ButtonClicked += Clicked;
            drop.ProblemChanged += report;
            report(drop.Problem);
        }
        catch (InkRendererException e)
        {
            // Without its window the Drop cannot show; the rest of the app runs on and says so.
            InkLog.Write(e.Message);
            report($"no Drop at all: {e.Message}");
        }
    }

    private void Update()
    {
        if (disposed)
        {
            return;
        }
        MakeDrop();
        if (held is { } pinned)
        {
            drop?.Update(pinned);
        }
        else if (text is not null)
        {
            drop?.Show(text, live);
        }
        else
        {
            drop?.Hide();
        }
        Changed?.Invoke();
    }

    /// <summary>UI thread. Stops the demo and destroys the Drop.</summary>
    public void Dispose()
    {
        disposed = true;
        demo?.Dispose();
        drop?.Dispose();
    }
}

/// <summary>
/// The Drop's focus check (windows/S3.4-CHECKLIST.md): with INK_DROP_DEMO=&lt;seconds&gt; the Drop
/// cycles through every state, one every &lt;seconds&gt;, while the owner types in another app. A
/// test harness, off unless that variable is set; its timer is the only one in the shell, and it
/// exists only for the check.
/// </summary>
internal sealed class DropDemo : IDisposable
{
    /// <summary>Each live state, then idle (the Drop hides and comes back, which must not take focus either).</summary>
    public static readonly InkState[] Sequence = [InkState.Dictating, InkState.Meeting, InkState.Blotting, InkState.Problem, InkState.Idle];

    private readonly DispatcherQueueTimer timer;
    private int index;

    /// <summary>The seconds per state INK_DROP_DEMO asks for, if it asks: a number of at least half a second.</summary>
    public static TimeSpan? Interval(string? raw) =>
        double.TryParse(raw, System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out var s)
            && double.IsFinite(s) && s >= 0.5
            ? TimeSpan.FromSeconds(s)
            : null;

    public DropDemo(DispatcherQueue ui, ShellInk ink, TimeSpan interval)
    {
        ink.Held = Sequence[0];
        timer = ui.CreateTimer();
        timer.Interval = interval;
        timer.IsRepeating = true;
        timer.Tick += (_, _) =>
        {
            index = (index + 1) % Sequence.Length;
            ink.Held = Sequence[index];
        };
        timer.Start();
    }

    public void Dispose() => timer.Stop();
}
