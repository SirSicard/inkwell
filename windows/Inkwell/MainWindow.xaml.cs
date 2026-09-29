using Microsoft.UI.Xaml;

namespace Inkwell;

public sealed partial class MainWindow : Window
{
    public MainWindow()
    {
        InitializeComponent();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBarArea);
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Inkwell.ico"));
    }

    /// <summary>UI thread. What the core's state is: its status line, and its version once ready.</summary>
    public void ShowStatus(CoreStatus status)
    {
        Starting.IsActive = status.Kind == CoreStatusKind.Starting;
        Starting.Visibility = Starting.IsActive ? Visibility.Visible : Visibility.Collapsed;
        Status.Text = status.Kind switch
        {
            CoreStatusKind.Starting => "Starting the core",
            CoreStatusKind.Ready => "Ready",
            CoreStatusKind.Failed => $"The core did not start: {status.Detail}",
            CoreStatusKind.Stopped => "The core stopped",
            _ => Status.Text,
        };
        Version.Text = status.Kind == CoreStatusKind.Ready ? $"core {status.Detail}" : "";
    }
}
