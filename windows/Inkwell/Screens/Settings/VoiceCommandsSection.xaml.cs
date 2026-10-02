// Settings > Voice commands. Every change goes through the model (the whole store is sent and the
// core's answer shown); this lays the rows out and keeps the wake word's box in step with it.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Windows.System;

namespace Inkwell.Screens;

public sealed partial class VoiceCommandsSection : UserControl
{
    private IReadOnlyList<VoiceCommandDraft>? shown;
    private string? shownWake;

    public VoiceCommandsSection(VoiceCommandsModel commands)
    {
        Model = commands ?? throw new ArgumentNullException(nameof(commands));
        InitializeComponent();
        Model.PropertyChanged += (_, _) => Render();
        Render();
    }

    public VoiceCommandsModel Model { get; }

    public static string StartOverTitle => PhraseLists.StartOverTitle;

    public static string StartOverHelp => PhraseLists.StartOverHelp;

    public static string FromImportText => PhraseLists.FromImportText;

    /// <summary>The picker's kinds, in its order (the model's Addable).</summary>
    private static CommandAction ActionAt(int index) => index == 1 ? CommandAction.ChangeStyle : CommandAction.InsertText;

    private void OnLoaded(object sender, RoutedEventArgs e) => Model.Load();

    private void Render()
    {
        RowsHost.IsEnabled = Model.Loaded;
        AddHost.IsEnabled = Model.Loaded;
        if (shownWake != Model.WakePrefix)
        {
            shownWake = Model.WakePrefix;
            Wake.Text = Model.WakePrefix;
        }
        SaveWake.IsEnabled = Model.CanSaveWakePrefix(Wake.Text);
        if (ReferenceEquals(shown, Model.Rows))
        {
            return;
        }
        shown = Model.Rows;
        var focus = RowFocus.Capture(Rows, XamlRoot);
        Rows.ItemsSource = Model.Rows.Select(row => new VoiceCommandItem(row)).ToList();
        RowFocus.Restore(Rows, focus, item => ((VoiceCommandItem)item).Id);
    }

    private void OnEnabledToggled(object sender, RoutedEventArgs e)
    {
        // Also raised when the model's state is shown: only the user's flip is a change.
        if (CommandsSwitch.IsOn != Model.Enabled)
        {
            Model.SetEnabled(CommandsSwitch.IsOn);
        }
    }

    private void OnWakeChanged(object sender, TextChangedEventArgs e) => SaveWake.IsEnabled = Model.CanSaveWakePrefix(Wake.Text);

    private void OnWakeKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Enter)
        {
            Model.SetWakePrefix(Wake.Text);
            e.Handled = true;
        }
    }

    private void OnSaveWake(object sender, RoutedEventArgs e) => Model.SetWakePrefix(Wake.Text);

    private VoiceCommandItem? RowOf(object sender) =>
        RowTag.Of(sender) is string id ? (Rows.ItemsSource as IEnumerable<VoiceCommandItem>)?.FirstOrDefault(r => r.Id == id) : null;

    private void OnToggled(object sender, RoutedEventArgs e)
    {
        if (sender is ToggleSwitch toggle && RowOf(sender) is VoiceCommandItem row && row.Enabled != toggle.IsOn)
        {
            Model.SetCommandEnabled(row.Id, toggle.IsOn);
        }
    }

    private void OnDelete(object sender, RoutedEventArgs e)
    {
        if (RowOf(sender) is VoiceCommandItem row)
        {
            Model.Delete(row.Id);
        }
    }

    private void OnNewActionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (NewValue is null)
        {
            return;
        }
        var text = ActionAt(NewAction.SelectedIndex) == CommandAction.InsertText ? "Text to type" : "Style or mode name";
        NewValue.PlaceholderText = text;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(NewValue, text);
    }

    private void OnNewChanged(object sender, TextChangedEventArgs e) =>
        Add.IsEnabled = VoiceCommandsModel.Phrases(NewTriggers.Text).Count > 0 && NewValue.Text.Trim().Length > 0;

    private void OnAdd(object sender, RoutedEventArgs e)
    {
        Model.Add(NewTriggers.Text, ActionAt(NewAction.SelectedIndex), NewValue.Text);
        NewTriggers.Text = "";
        NewValue.Text = "";
    }

    private void OnStartOver(object sender, RoutedEventArgs e) => Model.StartOver();
}
