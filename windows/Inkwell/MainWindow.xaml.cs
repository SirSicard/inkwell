using Inkwell.Core;
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

    /// <summary>UI thread. The window's ink follows the shell's ink state.</summary>
    internal void ShowInk(ShellInk ink)
    {
        Ink.State = ink.State;
        ink.Changed += () => Ink.State = ink.State;
    }

    /// <summary>UI thread. Why the Drop cannot draw its ink (it shows a plain panel meanwhile), or null once it draws again.</summary>
    internal void ShowInkFailure(string? failure)
    {
        InkStatus.Text = failure is null ? "" : $"The Drop is showing a plain panel: {failure}";
        InkStatus.Visibility = failure is null ? Visibility.Collapsed : Visibility.Visible;
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
            CoreStatusKind.MismatchedBuild => $"This shell and its core are from different builds (a {status.Detail} event did not decode)",
            CoreStatusKind.Stopped => "The core stopped",
            _ => Status.Text,
        };
        Version.Text = status.Kind == CoreStatusKind.Ready ? $"core {status.Detail}" : "";
    }
}
