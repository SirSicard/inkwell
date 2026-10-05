// The app: one window and a tray icon over the core. Closing the window hides it; the tray's Quit
// stops the core, then the app. The screens' models (ScreenModels) follow the core's events after
// the store; each route's screen is made from them (Screens.cs). The Drop follows meetings,
// dictation and the offer to record a call through DropModel after the store too; its buttons
// answer through the meetings model. The look is Glow's (GlowTheme): the window, the Drop and the
// tray icon's state dot follow the appearance settings. Before any of it, Microsoft's terms
// (TermsStep, TermsWindow): until they are agreed to, nothing else is made, shown or started.
//
// The tray icon and the window's taskbar button show the state (LiveIconHost: dictating in your
// colour, recording in theirs, the final pass's progress, a problem in the alert
// colour) and the tray's menu is made when it opens (TrayMenu): a left click opens the window. The automatic update check, when on, runs once here at launch.
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
    /// <summary>Held for the app's life: its hook follows what covers the window.</summary>
    private WindowCover? windowCover;
    private CoreController? core;
    private ScreenModels? screens;
    private ShellInk? ink;
    private GlowTheme? theme;
    private DropModel? dropModel;
    /// <summary>The tray icon's and the taskbar button's live state (made with the tray icon).</summary>
    private LiveIconHost? liveIcon;
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
        // The ink's lines (its GPU, its failures) too, still to the trace as before.
        var trace = InkLog.Write;
        InkLog.Write = line =>
        {
            trace(line);
            log.Write("ink", line);
        };
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
        var shownWindow = window;
        var made = new AppScreens(core.Store, models, router, glow, shownInk)
        {
            WindowHandle = () => (nint)Microsoft.UI.Win32Interop.GetWindowFromWindowId(shownWindow.AppWindow.Id),
        };
        window.Attach(core.Store, router, made.Screen, made.Search, models.Meetings, models.Owed);
        made.AttachFirstRun(window.Content as FrameworkElement);
        window.ShowMilestones(models.Stats, made.Presence);
        // Up next's minute redraws only while the window is on screen (rule 9): shown, not
        // minimised, and not hidden behind other windows (WindowCover).
        var cover = new WindowCover((nint)Microsoft.UI.Win32Interop.GetWindowFromWindowId(window.AppWindow.Id));
        windowCover = cover;
        cover.Changed += covered => made.Presence.Update(window.AppWindow.IsVisible, Minimized(window), occlusionVisible: !covered);
        // Uncovered, the orb at rest goes to a new spot, as on coming on screen.
        cover.Uncovered += window.WindowUncovered;
        window.VisibilityChanged += (_, e) => made.Presence.Update(e.Visible, Minimized(window), occlusionVisible: !cover.Covered);
        window.AppWindow.Changed += (sender, e) =>
        {
            if (e.DidPresenterChange || e.DidVisibilityChange)
            {
                made.Presence.Update(sender.IsVisible, Minimized(window), occlusionVisible: !cover.Covered);
                // A shortcut being recorded is the window's: hidden or minimised, it is cancelled.
                if (!sender.IsVisible || Minimized(window))
                {
                    models.Recorder.Cancel();
                }
            }
            window.FrameChanged();
            if (e.DidPositionChange || e.DidSizeChange)
            {
                window.PlaceChanged();
            }
        };
        // Coming back to the app re-checks what may have changed outside it (permissions, the keys).
        window.Activated += (_, e) =>
        {
            if (e.WindowActivationState != WindowActivationState.Deactivated)
            {
                models.AppBecameActive();
                window.WindowActivated();
                liveIcon?.AppActive();
            }
            else
            {
                // The recorder takes this window's keys only: losing focus cancels it, so dictation
                // never stays paused behind a recorder nobody sees.
                models.Recorder.Cancel();
            }
        };
        tray = new TrayIcon(1, IconPath, "Inkwell");
        tray.Selected += (_, _) => ShowWindow();
        tray.ContextMenu += (_, e) => e.Flyout = TrayMenuFlyout();
        tray.IsVisible = true;
        liveIcon = new LiveIconHost(
            (nint)Win32Interop.GetWindowFromWindowId(window.AppWindow.Id), drop, store, glow, tray, IconPath, window.DispatcherQueue,
            start =>
            {
                if (start)
                {
                    models.Meetings.RecordNow();
                }
                else
                {
                    models.Meetings.Stop();
                }
            });
        liveIcon.InkProblem(inkProblem);
        window.Activate();
        window.FitToWorkArea();
        // On screen from the start: the window's own change events may not come for the first show.
        made.Presence.Update(window.AppWindow.IsVisible, Minimized(window), occlusionVisible: !cover.Covered);
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

    /// <summary>
    /// UI thread. The Drop's problem, where it stays seen: the window's status line, and the tray
    /// icon's tooltip, which is there while the window is hidden.
    /// </summary>
    private void InkProblem(string? problem)
    {
        inkProblem = problem;
        window?.ShowInkFailure(problem);
        liveIcon?.InkProblem(problem);
    }

    /// <summary>
    /// The tray icon's tooltip, which Narrator reads: the state (LiveIcon.Spoken), or what stops the
    /// Drop (Windows keeps 128 characters).
    /// </summary>
    internal static string TrayTooltip(string? problem, DropInk state)
    {
        if (problem is null)
        {
            return LiveIcon.Spoken(state);
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
            liveIcon?.Dispose();
            liveIcon = null;
            windowCover?.Dispose();
            windowCover = null;
            tray?.Dispose();
            tray = null;
            window.Close();
            Exit();
        });
    }
}
