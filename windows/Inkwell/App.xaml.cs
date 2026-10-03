// The app: one window and a tray icon over the core. Closing the window hides it; the tray's Quit
// stops the core, then the app. The screens' models (ScreenModels) follow the core's events after
// the store; each route's screen is made from them (Screens.cs). The Drop follows meetings,
// dictation and the offer to record a call through DropModel after the store too; its buttons
// answer through the meetings model. The look is Glow's (GlowTheme): the window, the Drop and the
// tray icon's state dot follow the appearance settings. Before any of it, Microsoft's terms
// (TermsStep, TermsWindow): until they are agreed to, nothing else is made, shown or started.
//
// The tray icon shows the state (idle, dictating in your colour, recording in theirs, a problem
// in the alert colour: TrayGlyph draws the dot) and its menu is made when it opens (TrayMenu): a
// left click opens the window. The automatic update check, when on, runs once here at launch.
//
// From the start, the screens' log and the core's log lines go to the local log in the library's
// folder (LocalLog, "logs"), and an exception that ends the app leaves a crash note there.
using Inkwell.Core;
using Inkwell.Core.Screens;
using Inkwell.Ink;
using Inkwell.Screens;
using Microsoft.UI;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using WinUIEx;

namespace Inkwell;

// The tray icon is disposed on Quit, the only way the app ends; an Application is never disposed.
[System.Diagnostics.CodeAnalysis.SuppressMessage("Reliability", "CA1001", Justification = "Disposed on Quit")]
public partial class App : Application
{
    /// <summary>The terms step's window, while it is up (held, as the main window is).</summary>
    private TermsWindow? terms;
    private MainWindow? window;
    private TrayIcon? tray;
    private CoreController? core;
    private ScreenModels? screens;
    private ShellInk? ink;
    private GlowTheme? theme;
    private DropModel? dropModel;
    /// <summary>The tray icon's state icons, made for the colours shown (TrayGlyph), by state.</summary>
    private readonly Dictionary<TrayState, nint> trayIcons = [];
    private TrayState? trayShown;
    /// <summary>What stops the Drop working now, or null: kept for the tray icon made after it.</summary>
    private string? inkProblem;
    private bool quitting;
    private readonly Router router = new();
    private readonly LocalLog? localLog = OpenLocalLog();

    public App()
    {
        InitializeComponent();
        // A crash note for whatever ends the app: the UI thread's exceptions, then any other thread's.
        UnhandledException += (_, e) => localLog?.WriteCrashNote(e.Exception, AppVersion.Release);
        AppDomain.CurrentDomain.UnhandledException += (_, e) =>
        {
            if (e.ExceptionObject is Exception exception)
            {
                localLog?.WriteCrashNote(exception, AppVersion.Release);
            }
        };
        // Not a crash in .NET, but nothing else would ever say it happened.
        TaskScheduler.UnobservedTaskException += (_, e) =>
            localLog?.Write("shell", $"a task failed and nothing waited for it ({e.Exception.InnerException?.GetType().Name ?? "unknown"})");
    }

    /// <summary>
    /// The local log in the library's folder, now taking the screens' log and the core's lines
    /// (stderr), or null when that folder is not known (the core then says why).
    /// </summary>
    private static LocalLog? OpenLocalLog()
    {
        LocalLog log;
        try
        {
            log = LocalLog.In(DataLocation.DataDirectory());
        }
        catch (IOException)
        {
            return null;
        }
        ScreenLog.Also = message => log.Write("shell", message);
        if (!CoreLogCapture.Start(line =>
            {
                var (source, text) = LocalLog.FromStderr(line);
                log.Write(source, text);
            }))
        {
            log.Write("shell", "the core's log lines could not be captured");
        }
        log.Write("shell", $"Inkwell {AppVersion.Release ?? "development build"} started");
        return log;
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        // A second start of the app on this library hands its activation here (Program), off the
        // UI thread: what is up shows, the terms or the window.
        var ui = Microsoft.UI.Dispatching.DispatcherQueue.GetForCurrentThread();
        Microsoft.Windows.AppLifecycle.AppInstance.GetCurrent().Activated += (_, _) => ui.TryEnqueue(() =>
        {
            if (terms is not null)
            {
                terms.Activate();
            }
            else
            {
                ShowWindow();
            }
        });
        var step = new TermsStep(TermsFile(), Launch, Exit);
        step.Launch();
        if (step.Showing)
        {
            terms = new TermsWindow(step);
            terms.Closed += (_, _) => terms = null;
            terms.Activate();
        }
    }

    /// <summary>
    /// Where the agreement to the terms is kept: in the library's folder, or null when that folder
    /// is not known (INK_DATA_DIR is not an absolute path; the core then says so when it starts).
    /// </summary>
    private static TermsRecord? TermsFile()
    {
        try
        {
            return new TermsRecord(Path.Combine(DataLocation.DataDirectory(), TermsRecord.FileName));
        }
        catch (IOException)
        {
            return null;
        }
    }

    /// <summary>Everything the app does, once the terms are agreed to: the window, the core, the screens and the tray.</summary>
    private void Launch()
    {
        window = new MainWindow();
        window.AppWindow.Closing += (_, e) =>
        {
            // The window hides; the app lives on in the tray until Quit.
            if (!quitting)
            {
                e.Cancel = true;
                window.AppWindow.Hide();
            }
        };
        core = new CoreController(window.DispatcherQueue);
        // The ink's pipeline compiles off the UI thread from here; the Drop waits, hidden.
        ink = new ShellInk(window.DispatcherQueue, InkProblem, action => screens?.Meetings.Perform(action));
        InkPanel.Clock = ink.Clock;
        window.ShowInk(ink);
        screens = AppScreens.Models(core, window.DispatcherQueue, new VelopackUpdater(Quit));
        var models = screens;
        // The look: the window's mode and colours, the Drop's pill and orb, the tray's dots.
        var glow = new GlowTheme(models.Appearance, window.DispatcherQueue);
        theme = glow;
        window.ShowTheme(glow);
        var shownInk = ink;
        glow.Changed += () =>
        {
            shownInk.SetLook(glow.Look, glow.DropLook, glow.AlwaysStill);
            ColoursChanged();
        };
        shownInk.SetLook(glow.Look, glow.DropLook, glow.AlwaysStill);
        // What the Drop says, after the store has taken each batch.
        var drop = new DropModel(
            new DispatcherWake(window.DispatcherQueue), () => models.Polish.HasWorkingEngine,
            () => models.Meetings.FailureOn(MeetingPlace.Drop), noSpeechModel: () => models.Catalogue.HasSpeechModel == false);
        dropModel = drop;
        var shellInk = ink;
        drop.Changed += () =>
        {
            shellInk.Show(drop.Line, drop.Ink);
            ShowTrayState();
        };
        var store = core.Store;
        var applying = false;
        // An answer sent again clears the Drop's failure line at once, not at the next batch (a
        // change during a batch is the batch's: the Drop takes it whole, after the screens).
        models.Meetings.PropertyChanged += (_, _) =>
        {
            if (!applying)
            {
                drop.Refresh(store);
            }
        };
        core.Observer = batch =>
        {
            applying = true;
            try
            {
                models.Apply(batch);
                models.LogUnshown(batch);
            }
            finally
            {
                applying = false;
            }
            drop.Apply(store, batch);
        };
        var made = new AppScreens(core.Store, models, router, glow, shownInk);
        window.Attach(core.Store, router, made.Screen, made.Search, models.Meetings, models.Owed);
        made.AttachFirstRun(window.Content as FrameworkElement);
        // Up next's minute redraws only while the window is on screen (rule 9).
        window.VisibilityChanged += (_, e) => made.Presence.Update(e.Visible, Minimized(window), occlusionVisible: true);
        window.AppWindow.Changed += (sender, e) =>
        {
            if (e.DidPresenterChange || e.DidVisibilityChange)
            {
                made.Presence.Update(sender.IsVisible, Minimized(window), occlusionVisible: true);
            }
            window.FrameChanged();
        };
        // Coming back to the app re-checks what may have changed outside it (permissions, the keys).
        window.Activated += (_, e) =>
        {
            if (e.WindowActivationState != WindowActivationState.Deactivated)
            {
                models.AppBecameActive();
            }
        };
        tray = new TrayIcon(1, IconPath, "Inkwell");
        tray.Selected += (_, _) => ShowWindow();
        tray.ContextMenu += (_, e) => e.Flyout = TrayMenuFlyout();
        tray.Tooltip = TrayTooltip(inkProblem);
        tray.IsVisible = true;
        ShowTrayState();
        window.Activate();
        window.FitToWorkArea();
        // On screen from the start: the window's own change events may not come for the first show.
        made.Presence.Update(window.AppWindow.IsVisible, Minimized(window), occlusionVisible: true);
        core.Start();
        // Once, at launch, when the user turned the automatic check on (never on a timer).
        _ = models.Updates.CheckAtLaunch();
    }

    private static string IconPath => Path.Combine(AppContext.BaseDirectory, "Assets", "Inkwell.ico");

    /// <summary>The tray icon's menu, made as it opens: its status line is read now, so nothing ticks.</summary>
    private MenuFlyout TrayMenuFlyout()
    {
        var menu = new MenuFlyout();
        // It opens in the icon's own window, outside the main window's tree: without this it takes
        // Windows' mode, not the one Inkwell shows (Light while Windows is Dark, say). That window's
        // backdrop follows Windows, so the menu gets the mode's own opaque background too.
        if (theme is not null)
        {
            var style = new Style(typeof(MenuFlyoutPresenter));
            style.Setters.Add(new Setter(FrameworkElement.RequestedThemeProperty, theme.Dark ? ElementTheme.Dark : ElementTheme.Light));
            var background = GlowTheme.ColorOf(Inkwell.Core.Glow.GlowRgb.From(Inkwell.Core.Glow.GlowScheme.Palette(theme.Dark).Background));
            style.Setters.Add(new Setter(Control.BackgroundProperty, new Microsoft.UI.Xaml.Media.SolidColorBrush(background)));
            menu.MenuFlyoutPresenterStyle = style;
        }
        if (core is null || screens is null)
        {
            return menu;
        }
        var store = core.Store;
        var models = screens;
        var elapsed = store.Meeting is { Stopping: false } && models.Live.StartedAt is not null ? models.Live.ElapsedMs() : (long?)null;
        menu.Items.Add(new MenuFlyoutItem { Text = TrayMenu.StatusLine(store, elapsed), IsEnabled = false });
        var (recordTitle, recordEnabled) = TrayMenu.RecordItem(store);
        var record = new MenuFlyoutItem { Text = recordTitle, IsEnabled = recordEnabled };
        record.Click += (_, _) =>
        {
            if (store.Meeting is null)
            {
                models.Meetings.RecordNow();
            }
            else
            {
                models.Meetings.Stop();
            }
        };
        menu.Items.Add(record);
        var dictation = new ToggleMenuFlyoutItem { Text = TrayMenu.DictationTitle, IsChecked = models.Dictation.IsOn };
        dictation.Click += (_, _) => models.Dictation.SetOn(dictation.IsChecked);
        menu.Items.Add(dictation);
        menu.Items.Add(new MenuFlyoutSeparator());
        var open = new MenuFlyoutItem { Text = TrayMenu.OpenTitle };
        open.Click += (_, _) => ShowWindow();
        menu.Items.Add(open);
        var settings = new MenuFlyoutItem { Text = TrayMenu.SettingsTitle };
        settings.Click += (_, _) => ShowSettings();
        menu.Items.Add(settings);
        menu.Items.Add(new MenuFlyoutSeparator());
        models.Startup.Refresh();
        var startup = new ToggleMenuFlyoutItem
        {
            Text = StartupModel.Title,
            IsChecked = models.Startup.IsOn,
            IsEnabled = models.Startup.Available,
        };
        if (models.Startup.Unavailable is string why)
        {
            ToolTipService.SetToolTip(startup, why);
        }
        startup.Click += (_, _) => models.Startup.SetOn(startup.IsChecked);
        menu.Items.Add(startup);
        var updates = new MenuFlyoutItem { Text = TrayMenu.CheckForUpdatesTitle };
        updates.Click += (_, _) =>
        {
            // The answer shows where the updates row is: Settings > General.
            ShowSettings();
            _ = models.Updates.CheckNow();
        };
        menu.Items.Add(updates);
        menu.Items.Add(new MenuFlyoutSeparator());
        var quit = new MenuFlyoutItem { Text = TrayMenu.QuitTitle };
        quit.Click += (_, _) => Quit();
        menu.Items.Add(quit);
        return menu;
    }

    private void ShowSettings()
    {
        ShowWindow();
        router.Open(Route.Settings);
    }

    /// <summary>The tray icon for the Drop's state now: its dot in the colours shown.</summary>
    private void ShowTrayState()
    {
        if (tray is null || dropModel is null)
        {
            return;
        }
        var state = TrayMenu.State(dropModel.Ink);
        if (state == trayShown)
        {
            return;
        }
        if (!trayIcons.TryGetValue(state, out var icon))
        {
            try
            {
                icon = TrayGlyph.Make(IconPath, DotColour(state));
            }
            catch (InkRendererException e)
            {
                // The icon keeps the state it showed; the tooltip and the Drop still say it.
                InkLog.Write(e.Message);
                return;
            }
            trayIcons[state] = icon;
        }
        tray.SetIcon(Win32Interop.GetIconIdFromIcon(icon));
        trayShown = state;
    }

    /// <summary>The colours changed: the icons are drawn again; the old ones go once the new one shows (the shell keeps its own copy).</summary>
    private void ColoursChanged()
    {
        var old = trayIcons.Values.ToList();
        trayIcons.Clear();
        trayShown = null;
        ShowTrayState();
        foreach (var icon in old)
        {
            TrayGlyph.Destroy(icon);
        }
    }

    /// <summary>A state's dot: your colour, theirs, or the alert colour; none at rest.</summary>
    private (float R, float G, float B)? DotColour(TrayState state)
    {
        if (theme is null)
        {
            return null;
        }
        return state switch
        {
            TrayState.Dictating => theme.Look.YouA,
            TrayState.Recording => theme.Look.ThemA,
            TrayState.Problem => theme.DropLook.Alert,
            _ => null,
        };
    }

    /// <summary>
    /// UI thread. The Drop's problem, where it stays seen: the window's status line, and the tray
    /// icon's tooltip, which is there while the window is hidden.
    /// </summary>
    private void InkProblem(string? problem)
    {
        inkProblem = problem;
        window?.ShowInkFailure(problem);
        if (tray is not null)
        {
            tray.Tooltip = TrayTooltip(problem);
        }
    }

    /// <summary>The tray icon's tooltip: "Inkwell", or what stops the Drop (Windows keeps 128 characters).</summary>
    internal static string TrayTooltip(string? problem)
    {
        if (problem is null)
        {
            return "Inkwell";
        }
        var text = $"Inkwell. The Drop: {problem}";
        return text.Length <= 127 ? text : string.Concat(text.AsSpan(0, 126), "\u2026");
    }

    private static bool Minimized(Window window) =>
        window.AppWindow.Presenter is Microsoft.UI.Windowing.OverlappedPresenter { State: Microsoft.UI.Windowing.OverlappedPresenterState.Minimized };

    private void ShowWindow()
    {
        // From the tray: where it was hidden, which may be a display since unplugged.
        window?.FitToWorkArea();
        window?.AppWindow.Show();
        window?.Activate();
    }

    /// <summary>Every quit path the app controls: the core stops (its models unloaded) before the process ends.</summary>
    private void Quit()
    {
        if (quitting || window is null || core is null)
        {
            return;
        }
        quitting = true;
        localLog?.Write("shell", "quitting");
        // Quitting is not skipping the first run; and what the screens hold unsaved (the notes line
        // under the caret) reaches the core before it stops.
        screens?.AppQuitting();
        screens?.FlushBeforeStop();
        window.AppWindow.Hide();
        core.Stop(() =>
        {
            ink?.Dispose();
            ink = null;
            tray?.Dispose();
            tray = null;
            foreach (var (_, icon) in trayIcons)
            {
                TrayGlyph.Destroy(icon);
            }
            trayIcons.Clear();
            window.Close();
            Exit();
        });
    }
}
