// Settings > Stats. The switch and the speed are the model's settings; a failed read or save is said
// under them. The model reads them at core.ready, so nothing loads here.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class StatsSettingsSection : UserControl
{
    public StatsSettingsSection(StatsModel stats)
    {
        Model = stats ?? throw new ArgumentNullException(nameof(stats));
        InitializeComponent();
        Render();
        Loaded += (_, _) => Model.PropertyChanged += OnChanged;
        Unloaded += (_, _) => Model.PropertyChanged -= OnChanged;
    }

    public StatsModel Model { get; }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    /// <summary>The speed as the model has it (a NumberBox has no one-way binding that keeps a typed value).</summary>
    private void Render()
    {
        if (TypingWpm.Value != Model.TypingWpm)
        {
            TypingWpm.Value = Model.TypingWpm;
        }
        AutomationProperties.SetItemStatus(TypingWpm, StatsModel.TypingSpoken(Model.TypingWpm));
    }

    // Also raised when the model's value is shown: only the user's flip is a change.
    private void OnCelebrateToggled(object sender, RoutedEventArgs e)
    {
        if (Celebrate.IsOn != Model.Celebrate)
        {
            Model.SetCelebrate(Celebrate.IsOn);
        }
    }

    private void OnTypingWpmChanged(NumberBox sender, NumberBoxValueChangedEventArgs args)
    {
        // Cleared or not a number: the speed shown goes back to the model's.
        if (double.IsNaN(args.NewValue))
        {
            sender.Value = Model.TypingWpm;
            return;
        }
        var wpm = (int)Math.Round(args.NewValue);
        if (wpm != Model.TypingWpm)
        {
            Model.SetTypingWpm(wpm);
        }
    }
}
