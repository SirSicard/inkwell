// The app: one window and a tray icon over the core. Closing the window hides it; the tray's Quit
// stops the core, then the app. The screens' models (ScreenModels) follow the core's events after
// the store; each route's screen is made from them (Screens.cs). The Drop follows meetings,
// dictation and the offer to record a call through DropModel after the store too; its buttons
// answer through the meetings model. Before any of it, Microsoft's terms (TermsStep, TermsWindow):
// until they are agreed to, nothing else is made, shown or started.
using Inkwell.Core.Screens;
using Inkwell.Screens;
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
    /// <summary>What stops the Drop working now, or null: kept for the tray icon made after it.</summary>
    private string? inkProblem;
    private bool quitting;
    private readonly Router router = new();

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
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
        // What the Drop says, after the store has taken each batch.
        var drop = new DropModel(
            new DispatcherWake(window.DispatcherQueue), () => models.Polish.HasWorkingEngine,
            () => models.Meetings.FailureOn(MeetingPlace.Drop));
        var shellInk = ink;
        drop.Changed += () => shellInk.Show(drop.Line, drop.Ink);
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
        var made = new AppScreens(core.Store, models, router);
        window.Attach(core.Store, router, made.Screen, made.InkZoneFoot(), made.Search);
        made.AttachFirstRun(window.Content as FrameworkElement);
        // Up next's minute redraws only while the window is on screen (rule 9).
        window.VisibilityChanged += (_, e) => made.Presence.Update(e.Visible, Minimized(window), occlusionVisible: true);
        window.AppWindow.Changed += (sender, e) =>
        {
            if (e.DidPresenterChange || e.DidVisibilityChange)
            {
                made.Presence.Update(sender.IsVisible, Minimized(window), occlusionVisible: true);
            }
        };
        // Coming back to the app re-checks what may have changed outside it (permissions, the keys).
        window.Activated += (_, e) =>
        {
            if (e.WindowActivationState != WindowActivationState.Deactivated)
            {
                models.AppBecameActive();
            }
        };
        tray = new TrayIcon(1, Path.Combine(AppContext.BaseDirectory, "Assets", "Inkwell.ico"), "Inkwell");
        tray.Selected += (_, _) => ShowWindow();
        tray.ContextMenu += (_, e) =>
        {
            var show = new MenuFlyoutItem { Text = "Show Inkwell" };
            show.Click += (_, _) => ShowWindow();
            var quit = new MenuFlyoutItem { Text = "Quit Inkwell" };
            quit.Click += (_, _) => Quit();
            var menu = new MenuFlyout();
            menu.Items.Add(show);
            menu.Items.Add(quit);
            e.Flyout = menu;
        };
        tray.Tooltip = TrayTooltip(inkProblem);
        tray.IsVisible = true;
        window.Activate();
        // On screen from the start: the window's own change events may not come for the first show.
        made.Presence.Update(window.AppWindow.IsVisible, Minimized(window), occlusionVisible: true);
        core.Start();
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
            window.Close();
            Exit();
        });
    }
}
