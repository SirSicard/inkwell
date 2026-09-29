// The ink the shell shows: one state for every surface that draws it (the Drop, an InkPanel in the
// window), the process's pipeline and frame clock, and the Drop itself. The Mac's ShellInk
// (mac/Sources/Inkwell/ShellInk.swift). What the core says is live arrives through Live (wired with
// dictation and meetings end to end); Held pins a state for the Drop's focus check
// (INK_DROP_DEMO). Nothing here polls.
//
// When the Drop cannot draw its ink (a lost device it is recovering from, a shader that does not
// compile, no window at all) the failure goes to the window through the callback the app gives,
// as the core's failures do (CoreController), as well as to the log; null says it draws again.
using Inkwell.Ink;
using Microsoft.UI.Dispatching;

namespace Inkwell;

internal sealed class ShellInk : IDisposable
{
    private readonly DropWindow? drop;
    private readonly DropDemo? demo;
    private InkState live = InkState.Idle;
    private InkState? held;

    /// <summary>The frame clock every ink in the process shares.</summary>
    public InkClock Clock { get; }

    /// <summary>The pipeline, compiled once off the UI thread from launch.</summary>
    public static InkPipelineLoader Loader => InkPipelineLoader.Shared;

    /// <summary>UI thread. Starts the pipeline's compile and makes the (hidden) Drop; <paramref name="showFailure"/> shows why the ink cannot draw (null: it draws).</summary>
    public ShellInk(DispatcherQueue ui, Action<string?> showFailure)
    {
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
        try
        {
            drop = new DropWindow(Loader, Clock);
            drop.Surface.FailureChanged += showFailure;
        }
        catch (InkRendererException e)
        {
            // Without its window the Drop cannot show; the rest of the app runs on and says so.
            InkLog.Write(e.Message);
            showFailure(e.Message);
        }
        if (DropDemo.Interval(Environment.GetEnvironmentVariable("INK_DROP_DEMO")) is { } interval)
        {
            demo = new DropDemo(ui, this, interval);
        }
    }

    /// <summary>What the ink shows now.</summary>
    public InkState State => held ?? live;

    /// <summary>The state changed (the window's inks follow it).</summary>
    public event Action? Changed;

    /// <summary>What the core says is live.</summary>
    public InkState Live
    {
        get => live;
        set
        {
            live = value;
            Update();
        }
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

    private void Update()
    {
        drop?.Update(State);
        Changed?.Invoke();
    }

    /// <summary>UI thread. Stops the demo and destroys the Drop.</summary>
    public void Dispose()
    {
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
