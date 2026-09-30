// The first run's terms step, in a window of its own (TermsWindow.xaml; the logic is TermsStep's).
// Agree starts the app, whose main window opens before this one closes; Quit, or closing this
// window, exits with nothing started.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;

namespace Inkwell;

public sealed partial class TermsWindow : Window
{
    private readonly TermsStep step;

    public TermsWindow(TermsStep step)
    {
        ArgumentNullException.ThrowIfNull(step);
        this.step = step;
        InitializeComponent();
        AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "Inkwell.ico"));
        AppWindow.Closing += (_, e) =>
        {
            // Closing the window unanswered is Quit.
            if (step.Showing)
            {
                e.Cancel = true;
                step.Quit();
            }
        };
    }

    private void OnAgree(object sender, RoutedEventArgs e)
    {
        step.Agree();
        Close();
    }

    private void OnQuit(object sender, RoutedEventArgs e) => step.Quit();
}
