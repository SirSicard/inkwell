// The app: one window and a tray icon over the core. Closing the window hides it; the tray's Quit
// stops the core, then the app.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
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
        window.Attach(core.Store, router, Screen, new StackPanel());
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
        core.Start();
    }

    /// <summary>A route's screen.</summary>
    private static UIElement Screen(Route route)
    {
        var title = new TextBlock { Text = route.Title(), Style = (Style)Current.Resources["InkScreenTitleStyle"] };
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level1);
        return new StackPanel { Padding = new Thickness(48, 38, 48, 28), Children = { title } };
    }

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
