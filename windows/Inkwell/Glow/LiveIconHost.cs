// The live icon on Windows (LiveIcon decides; this shows): the tray icon and the main window's
// taskbar button follow the Drop's state, the final pass's steps, the user's colours and the motion
// settings. Nothing polls: each follows an event (the Drop's change, the store's, the theme's,
// Windows' animation setting), and the pulse's timer runs only while LiveIcon asks for it. While
// the screen is locked or the display is off (WindowHook) nothing ticks or draws; the app coming to
// the front means someone is there, so a missed unlock never leaves the icon still for good.
//
//   the tray      the Halo rim mark with the state's dot or ring (TrayGlyph.Tray); Narrator reads
//                 its tooltip, which says the state (or what stops the Drop)
//   the taskbar   an overlay badge in the state's colour with its description; the progress bar
//                 for the final pass (indeterminate until its first step) and the error state for
//                 a problem; the thumbnail toolbar's Record / Stop
//
// The pictures are made once per look, colour and step of the breath, and kept (fourteen for a
// breath), so a recording's 7 frames a second make no new icons after its first two seconds.
using Inkwell.Core;
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Inkwell.Ink;
using Microsoft.UI;
using Microsoft.UI.Dispatching;
using WinUIEx;

namespace Inkwell;

internal sealed class LiveIconHost : IDisposable
{
    private readonly LiveIcon icon;
    private readonly DropModel drop;
    private readonly CoreStore store;
    private readonly GlowTheme theme;
    private readonly TrayIcon tray;
    private readonly string iconPath;
    private readonly WindowHook hook;
    private readonly TaskbarButton taskbar;
    private readonly TraySurface traySurface;
    private readonly TaskbarSurface taskbarSurface;
    private bool locked;
    private bool displayOff;
    private string? inkProblem;

    public LiveIconHost(nint window, DropModel drop, CoreStore store, GlowTheme theme, TrayIcon tray, string iconPath, DispatcherQueue ui, Action<bool> record)
    {
        this.drop = drop;
        this.store = store;
        this.theme = theme;
        this.tray = tray;
        this.iconPath = iconPath;
        icon = new LiveIcon(new DispatcherTicker(ui));
        hook = new WindowHook(window);
        taskbar = new TaskbarButton(window);
        traySurface = new TraySurface(this);
        taskbarSurface = new TaskbarSurface(this);
        hook.TaskbarButtonCreated += () =>
        {
            // Made again (Explorer restarted): the badge, progress and buttons are put back.
            if (taskbar.Connect())
            {
                taskbarSurface.Reset();
                taskbarSurface.Show(icon.Frame);
                ShowButton();
            }
        };
        hook.ThumbnailClicked += id =>
        {
            if (id == TaskbarButton.RecordButtonId)
            {
                record(store.Meeting is null);
            }
        };
        hook.Locked += value =>
        {
            locked = value;
            Awake();
        };
        hook.DisplayOn += on =>
        {
            displayOff = !on;
            Awake();
        };
        // The button may already exist: connect now too (the message comes again if not).
        if (taskbar.Connect())
        {
            ShowButton();
        }
        icon.Attach(traySurface);
        icon.Attach(taskbarSurface);
        drop.Changed += Follow;
        store.PropertyChanged += (_, _) =>
        {
            Follow();
            ShowButton();
        };
        theme.Changed += Follow;
        SystemMotion.Changed += Follow;
        Follow();
    }

    /// <summary>Frames drawn since launch (the energy budget's count).</summary>
    public int Frames => icon.Frames;

    /// <summary>The app came to the front: someone is at an unlocked, awake screen.</summary>
    public void AppActive()
    {
        locked = false;
        displayOff = false;
        Awake();
    }

    /// <summary>What stops the Drop working now, or null: the tray's tooltip says it.</summary>
    public void InkProblem(string? problem)
    {
        inkProblem = problem;
        tray.Tooltip = App.TrayTooltip(problem, drop.Ink);
    }

    private void Awake() => icon.SetAwake(!locked && !displayOff);

    /// <summary>The look for the state now, in the colours shown: drawing only what changed (LiveIcon.Update).</summary>
    private void Follow()
    {
        var state = drop.Ink;
        // The store lets the meeting go a moment before the Drop leaves the final pass: its last
        // progress stays, rather than one frame of a dashed ring before rest.
        var progress = state != DropInk.Blotting ? null
            : store.Meeting is null && icon.Frame.Look is LiveIconLook.Ring shown ? shown.Progress
            : LiveIcon.FinalPassProgress(store.Meeting);
        var still = theme.AlwaysStill || !SystemMotion.AnimationsEnabled;
        icon.Update(LiveIconLook.For(state, progress, still), Colours());
        tray.Tooltip = App.TrayTooltip(inkProblem, state);
    }

    /// <summary>The night mode's colours: the icon's plate is night in either mode.</summary>
    private LiveIconColours Colours()
    {
        var night = theme.Appearance.Colours(dark: true);
        return new LiveIconColours(night.You, night.Them, GlowRgb.From(GlowScheme.Palette(true).Alert));
    }

    /// <summary>The thumbnail toolbar's one button: Record while nothing records, Stop while something does.</summary>
    private void ShowButton()
    {
        var (_, enabled) = TrayMenu.RecordItem(store);
        var stop = store.Meeting is not null;
        taskbarSurface.Button(stop, enabled);
    }

    private static (float R, float G, float B) Tuple(GlowRgb c) => ((float)c.R, (float)c.G, (float)c.B);

    public void Dispose()
    {
        icon.Detach(traySurface);
        icon.Detach(taskbarSurface);
        traySurface.Dispose();
        taskbarSurface.Dispose();
        taskbar.Dispose();
        hook.Dispose();
    }

    /// <summary>Icons kept by what they show; let go of when their colours change.</summary>
    private sealed class IconCache : IDisposable
    {
        private readonly Dictionary<string, nint> icons = [];
        private LiveIconColours? colours;

        public nint Get(LiveIconColours forColours, string key, Func<nint> make)
        {
            if (forColours != colours)
            {
                Dispose();
                colours = forColours;
            }
            if (!icons.TryGetValue(key, out var made))
            {
                made = make();
                icons[key] = made;
            }
            return made;
        }

        public void Dispose()
        {
            // The shell keeps its own copy of what it shows, so these can go.
            foreach (var made in icons.Values)
            {
                TrayGlyph.Destroy(made);
            }
            icons.Clear();
        }
    }

    /// <summary>The breath's strength as a key: it repeats every fourteen ticks.</summary>
    private static string StrengthKey(double strength) => strength.ToString("F3", System.Globalization.CultureInfo.InvariantCulture);

    private sealed class TraySurface(LiveIconHost host) : ILiveIconSurface, IDisposable
    {
        private readonly IconCache cache = new();

        public void Show(LiveIconFrame frame)
        {
            var (look, colour, strength, progress) = frame.Look switch
            {
                LiveIconLook.Glow glow => (GlyphLook.Dot, frame.Colours.Colour(glow.Tone), 1.0, (double?)null),
                LiveIconLook.Pulse pulse => (GlyphLook.Dot, frame.Colours.Colour(pulse.Tone), frame.Strength, null),
                LiveIconLook.Ring ring => (GlyphLook.Ring, frame.Colours.Them, 1.0, ring.Progress),
                _ => (GlyphLook.Rest, default(GlowRgb), 1.0, null),
            };
            var key = $"{look}:{colour}:{StrengthKey(strength)}:{progress}";
            try
            {
                var made = cache.Get(frame.Colours, key, () => TrayGlyph.Tray(host.iconPath, look, Tuple(colour), strength, progress));
                host.tray.SetIcon(Win32Interop.GetIconIdFromIcon(made));
            }
            catch (InkRendererException e)
            {
                // The icon keeps what it showed; the tooltip and the Drop still say the state.
                InkLog.Write(e.Message);
            }
        }

        public void Dispose() => cache.Dispose();
    }

    private sealed class TaskbarSurface(LiveIconHost host) : ILiveIconSurface, IDisposable
    {
        private readonly IconCache cache = new();
        private readonly IconCache buttons = new();
        private string? badgeKey;
        private (TaskbarProgress State, double Done)? progressShown;

        /// <summary>The button was made again: everything is shown afresh.</summary>
        public void Reset()
        {
            badgeKey = null;
            progressShown = null;
        }

        public void Show(LiveIconFrame frame)
        {
            var state = host.drop.Ink;
            var (tone, strength) = frame.Look switch
            {
                LiveIconLook.Glow glow => (glow.Tone, 1.0),
                LiveIconLook.Pulse pulse => (pulse.Tone, frame.Strength),
                LiveIconLook.Ring => (LiveIconTone.Them, 1.0),
                _ => ((LiveIconTone?)null, 1.0),
            };
            var key = tone is LiveIconTone k ? $"{k}:{frame.Colours.Colour(k)}:{StrengthKey(strength)}" : "none";
            if (key != badgeKey)
            {
                badgeKey = key;
                try
                {
                    var made = tone is LiveIconTone t
                        ? cache.Get(frame.Colours, key, () => TrayGlyph.Badge(Tuple(frame.Colours.Colour(t)), strength))
                        : 0;
                    host.taskbar.Overlay(made, LiveIcon.OverlayText(state));
                }
                catch (InkRendererException e)
                {
                    InkLog.Write(e.Message);
                }
            }
            // The final pass's progress; a problem's error state; nothing otherwise.
            (TaskbarProgress, double) progress = frame.Look switch
            {
                LiveIconLook.Ring { Progress: double p } => (TaskbarProgress.Normal, p),
                LiveIconLook.Ring => (TaskbarProgress.Indeterminate, 0),
                LiveIconLook.Glow { Tone: LiveIconTone.Alert } => (TaskbarProgress.Error, 1),
                _ => (TaskbarProgress.None, 0),
            };
            if (progress != progressShown)
            {
                progressShown = progress;
                host.taskbar.Progress(progress.Item1, progress.Item2);
            }
        }

        /// <summary>The thumbnail toolbar's button, in their colour.</summary>
        public void Button(bool stop, bool enabled)
        {
            var colours = host.Colours();
            try
            {
                var made = buttons.Get(colours, stop ? "stop" : "record", () => TrayGlyph.Thumb(stop, Tuple(colours.Them)));
                host.taskbar.Button(made, stop ? LiveIcon.StopButton : LiveIcon.RecordButton, enabled);
            }
            catch (InkRendererException e)
            {
                InkLog.Write(e.Message);
            }
        }

        public void Dispose()
        {
            cache.Dispose();
            buttons.Dispose();
        }
    }

    /// <summary>The pulse's clock: a dispatcher timer on the UI thread, made only while it runs.</summary>
    private sealed class DispatcherTicker(DispatcherQueue ui) : ILiveIconTicker
    {
        private DispatcherQueueTimer? timer;

        public bool Running => timer is not null;

        public void Start(TimeSpan interval, Action tick)
        {
            Cancel();
            timer = ui.CreateTimer();
            timer.Interval = interval;
            timer.IsRepeating = true;
            timer.Tick += (_, _) => tick();
            timer.Start();
        }

        public void Cancel()
        {
            timer?.Stop();
            timer = null;
        }
    }
}
