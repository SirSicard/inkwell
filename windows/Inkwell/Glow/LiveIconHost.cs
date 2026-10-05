// The live icon on Windows (LiveIcon decides; this shows): the tray icon and the main window's
// taskbar button follow the Drop's state, the final pass's steps and the user's colours. Nothing
// polls and nothing ticks: each follows an event (the Drop's change, the store's, the theme's), and
// a recording is held still (LiveIconLook.OnShell), as each frame on these surfaces is a call into
// Explorer. While the screen is locked or the display is off (WindowHook)
// nothing draws; the app coming to the front means someone is there, so a missed unlock never
// leaves the icon stale for good.
//
//   the tray      the Halo rim mark with the state's dot or ring (TrayGlyph.Tray); Narrator reads
//                 its tooltip, which says the state (or what stops the Drop)
//   the taskbar   an overlay badge in the state's colour with its description; the progress bar
//                 for the final pass (indeterminate until its first step) and the error state for
//                 a problem; the thumbnail toolbar's Record / Stop
//
// The pictures are made once per look and colour, and kept.
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
    /// <summary>The window's messages (null if the window could not be watched: no thumbnail clicks, and the live icon cannot hear the lock).</summary>
    private readonly WindowHook? hook;
    private readonly TaskbarButton taskbar;
    private readonly TraySurface traySurface;
    private readonly TaskbarSurface taskbarSurface;
    private bool locked;
    private bool displayOff;
    private string? inkProblem;
    private string? tooltipShown;
    private bool disposed;
    private readonly System.ComponentModel.PropertyChangedEventHandler storeChanged;
    /// <summary>Failures logged, by what failed: each once, however often a frame retries it.</summary>
    private readonly HashSet<string> logged = [];

    public LiveIconHost(nint window, DropModel drop, CoreStore store, GlowTheme theme, TrayIcon tray, string iconPath, DispatcherQueue ui, Action<bool> record)
    {
        this.drop = drop;
        this.store = store;
        this.theme = theme;
        this.tray = tray;
        this.iconPath = iconPath;
        icon = new LiveIcon(new DispatcherTicker(ui));
        try
        {
            hook = new WindowHook(window);
        }
        catch (InkRendererException e)
        {
            // The icons still follow the state: only the thumbnail's clicks and the lock are lost.
            InkLog.Write(e.Message);
        }
        taskbar = new TaskbarButton(window);
        traySurface = new TraySurface(this);
        taskbarSurface = new TaskbarSurface(this);
        storeChanged = (_, _) =>
        {
            Follow();
            ShowButton();
        };
        if (hook is not null)
        {
            Listen(hook, record);
        }
        // The button may already exist: connect now too (the message comes again if not).
        if (taskbar.Connect())
        {
            ShowButton();
        }
        icon.Attach(traySurface);
        icon.Attach(taskbarSurface);
        drop.Changed += Follow;
        store.PropertyChanged += storeChanged;
        theme.Changed += Follow;
        Follow();
    }

    private void Listen(WindowHook hook, Action<bool> record)
    {
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
        Tooltip(App.TrayTooltip(problem, drop.Ink));
    }

    /// <summary>The tray's tooltip, told to the shell only when it changes (the store changes many times a second while recording).</summary>
    private void Tooltip(string text)
    {
        if (text != tooltipShown)
        {
            tooltipShown = text;
            tray.Tooltip = text;
        }
    }

    /// <summary>A surface's failure: logged once for each kind, never into the pulse's timer or the core's batch.</summary>
    private void Failed(string what, Exception e)
    {
        if (logged.Add($"{what}:{e.GetType().Name}"))
        {
            // By its type only: an exception's message never reaches the log.
            InkLog.Write($"the live icon couldn't {what}: {e.GetType().Name}");
        }
    }

    private void Awake() => icon.SetAwake(!locked && !displayOff);

    /// <summary>The look for the state now, in the colours shown: drawing only what changed (LiveIcon.Update).</summary>
    private void Follow()
    {
        if (disposed)
        {
            return;
        }
        var state = drop.Ink;
        // The store lets the meeting go a moment before the Drop leaves the final pass: its last
        // progress stays, rather than one frame of a dashed ring before rest.
        var progress = state != DropInk.Blotting ? null
            : store.Meeting is null && icon.Frame.Look is LiveIconLook.Ring shown ? shown.Progress
            : LiveIcon.FinalPassProgress(store.Meeting);
        // Still in every state, so the badge and the tray icon change only with the state: a
        // breath would be seven calls a second into Explorer (SetOverlayIcon; Shell_NotifyIcon,
        // which TrayIcon.SetIcon makes on every call) on this thread, and a hung Explorer would
        // stall the app. Neither can move without them, so nothing on them breathes and no timer
        // runs (Always still and Windows' animation setting have nothing left to hold still here).
        icon.Update(LiveIconLook.OnShell(state, progress), Colours());
        Tooltip(App.TrayTooltip(inkProblem, state));
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
        if (disposed)
        {
            return;
        }
        var (_, enabled) = TrayMenu.RecordItem(store);
        var stop = store.Meeting is not null;
        taskbarSurface.Button(stop, enabled);
    }

    private static (float R, float G, float B) Tuple(GlowRgb c) => ((float)c.R, (float)c.G, (float)c.B);

    public void Dispose()
    {
        disposed = true;
        drop.Changed -= Follow;
        store.PropertyChanged -= storeChanged;
        theme.Changed -= Follow;
        icon.Detach(traySurface);
        icon.Detach(taskbarSurface);
        traySurface.Dispose();
        taskbarSurface.Dispose();
        taskbar.Dispose();
        hook?.Dispose();
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
            catch (Exception e)
            {
                // The icon keeps what it showed; the tooltip and the Drop still say the state.
                host.Failed("draw the tray icon", e);
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
        private (bool Stop, bool Enabled, LiveIconColours Colours)? buttonShown;

        /// <summary>The button was made again: everything is shown afresh.</summary>
        public void Reset()
        {
            badgeKey = null;
            progressShown = null;
            buttonShown = null;
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
            // The description is part of what is shown: a recording held still and the final pass
            // have the same badge, but not the same words for Narrator.
            var description = LiveIcon.OverlayText(state);
            var picture = tone is LiveIconTone k ? $"{k}:{frame.Colours.Colour(k)}:{StrengthKey(strength)}" : "none";
            var key = $"{picture}|{description}";
            if (key != badgeKey)
            {
                try
                {
                    var made = tone is LiveIconTone t
                        ? cache.Get(frame.Colours, picture, () => TrayGlyph.Badge(Tuple(frame.Colours.Colour(t)), strength))
                        : 0;
                    // Shown only once the shell took it: a failure is tried again with the next frame.
                    if (host.taskbar.Overlay(made, description))
                    {
                        badgeKey = key;
                    }
                }
                catch (Exception e)
                {
                    host.Failed("draw the taskbar badge", e);
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
            if (progress != progressShown && host.taskbar.Progress(progress.Item1, progress.Item2))
            {
                progressShown = progress;
            }
        }

        /// <summary>The thumbnail toolbar's button, in their colour.</summary>
        public void Button(bool stop, bool enabled)
        {
            var colours = host.Colours();
            // Told to the shell only when it changes: the store changes many times a second while recording.
            if (buttonShown == (stop, enabled, colours))
            {
                return;
            }
            try
            {
                var made = buttons.Get(colours, stop ? "stop" : "record", () => TrayGlyph.Thumb(stop, Tuple(colours.Them)));
                if (host.taskbar.Button(made, stop ? LiveIcon.StopButton : LiveIcon.RecordButton, enabled))
                {
                    buttonShown = (stop, enabled, colours);
                }
            }
            catch (Exception e)
            {
                host.Failed("show the thumbnail button", e);
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
