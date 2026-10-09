// Settings > General. Start with Windows reads what Windows holds when the section shows and
// writes only when the switch is pressed; the updates row follows its model (UpdatesModel), which
// changes only when the user presses its button, or once at launch while the automatic check is
// on. Nothing here ticks.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class GeneralSection : UserControl
{
    private readonly StartupModel startup;
    private readonly UpdatesModel updates;
    private bool rendering;

    /// <param name="import02">Inkwell 0.2's history card.</param>
    public GeneralSection(StartupModel startup, UpdatesModel updates, UIElement? import02 = null)
    {
        ArgumentNullException.ThrowIfNull(startup);
        ArgumentNullException.ThrowIfNull(updates);
        this.startup = startup;
        this.updates = updates;
        InitializeComponent();
        StartupLabel.Text = StartupModel.Title;
        AutomationProperties.SetName(StartupSwitch, StartupModel.Title);
        AutoCheckLabel.Text = UpdatesModel.AutoCheckTitle;
        AutomationProperties.SetName(AutoCheckSwitch, UpdatesModel.AutoCheckTitle);
        ImportHost.Content = import02;
        startup.PropertyChanged += (_, _) => Render();
        updates.PropertyChanged += (_, _) => Render();
        Loaded += (_, _) =>
        {
            startup.Refresh();
            Render();
        };
        Render();
    }

    private void Render()
    {
        rendering = true;
        try
        {
            StartupSwitch.IsEnabled = startup.Available;
            StartupSwitch.IsOn = startup.IsOn;
            var startupCaption = startup.Unavailable ?? StartupModel.Caption;
            StartupCaption.Text = startupCaption;
            AutomationProperties.SetHelpText(StartupSwitch, startupCaption);
            Show(StartupFailure, startup.Failure);

            AutoCheckSwitch.IsEnabled = updates.CanAutoCheck;
            AutoCheckSwitch.IsOn = updates.AutoCheck;
            AutoCheckCaption.Text = updates.CanAutoCheck ? UpdatesModel.AutoCheckCaption : UpdatesModel.OffText;
            Show(AutoCheckFailure, updates.AutoCheckFailure);

            UpdatesButton.Visibility = updates.ActionTitle is null ? Visibility.Collapsed : Visibility.Visible;
            UpdatesButton.Content = updates.ActionTitle;
            UpdatesButton.IsEnabled = updates.CanAct;
            // A copy that does not update itself says so once, by the switch.
            Show(UpdatesLine, updates.State == UpdateState.Off ? null : updates.Line);
            UpdatesRow.Visibility = UpdatesButton.Visibility == Visibility.Visible || UpdatesLine.Visibility == Visibility.Visible
                ? Visibility.Visible
                : Visibility.Collapsed;
        }
        finally
        {
            rendering = false;
        }
    }

    private static void Show(TextBlock block, string? text)
    {
        block.Text = text ?? "";
        block.Visibility = text is null ? Visibility.Collapsed : Visibility.Visible;
    }

    private void OnStartupToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering)
        {
            startup.SetOn(StartupSwitch.IsOn);
            Render();
        }
    }

    private void OnAutoCheckToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering)
        {
            updates.SetAutoCheck(AutoCheckSwitch.IsOn);
            Render();
        }
    }

    private async void OnUpdatesAct(object sender, RoutedEventArgs e)
    {
        try
        {
            await updates.Act().ConfigureAwait(true);
        }
        catch (Exception failure)
        {
            // Act says its own failures on the row; anything else is logged by kind.
            ScreenLog.System.Write($"the updates row failed ({failure.GetType().Name})");
        }
    }
}
