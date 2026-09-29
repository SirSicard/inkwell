// The app: one window and a tray icon over the core. Closing the window hides it; the tray's Quit
// stops the core, then the app. The screens' models (ScreenModels) follow the core's events after
// the store; each route's screen is made from them (Screens.cs).
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using WinUIEx;

namespace Inkwell;

// The tray icon is disposed on Quit, the only way the app ends; an Application is never disposed.
[System.Diagnostics.CodeAnalysis.SuppressMessage("Reliability", "CA1001", Justification = "Disposed on Quit")]
public partial class App : Application
{
    private MainWindow? window;
    private TrayIcon? tray;
    private CoreController? core;
    private ScreenModels? screens;
    private bool quitting;
    private readonly Router router = new();

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
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
        screens = AppScreens.Models(core, window.DispatcherQueue);
        var models = screens;
        core.Observer = batch =>
        {
            models.Apply(batch);
            models.LogUnshown(batch);
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
        tray.IsVisible = true;
        window.Activate();
        // On screen from the start: the window's own change events may not come for the first show.
        made.Presence.Update(window.AppWindow.IsVisible, Minimized(window), occlusionVisible: true);
        core.Start();
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
            tray?.Dispose();
            tray = null;
            window.Close();
            Exit();
        });
    }
}
