// Settings > Stats. The switches, the speed and the rest days are the model's settings; a failed
// read or save is said under them. The model reads them at core.ready; the pause's state is in the
// numbers, counted once when the section first shows (StatsModel.SettingsAppeared).
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace Inkwell.Screens;

public sealed partial class StatsSettingsSection : UserControl
{
    public StatsSettingsSection(StatsModel stats)
    {
        Model = stats ?? throw new ArgumentNullException(nameof(stats));
        InitializeComponent();
        Render();
        Loaded += (_, _) =>
        {
            Model.PropertyChanged += OnChanged;
            Model.SettingsAppeared();
            Render();
        };
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
        RenderRestDays();
        var paused = Model.Counted?.Dictation.StreakPausedSince;
        // Unknown until counted: never a Pause offered over a pause that may be running.
        var known = Model.Counted is not null;
        PauseButton.Content = paused is null ? "Pause" : "Resume";
        AutomationProperties.SetName(PauseButton, paused is null ? "Pause the streak" : "Resume the streak");
        // Enabled while a change is in flight (the model ignores a second), so the keyboard's focus
        // stays on it.
        PauseButton.IsEnabled = known;
        PauseCaption.Text = known
            ? StatsModel.PauseCaption(paused, Model.Culture)
            : StatsModel.PauseUnknown(Model.LoadState == StatsModel.Load.Failed);
        AutomationProperties.SetHelpText(PauseButton, PauseCaption.Text);
        var failure = Model.StreakChangeFailed is { } failed ? StatsModel.StreakFailure(failed) : "";
        if (PauseFailure.Text != failure)
        {
            PauseFailure.Text = failure;
            if (failure.Length > 0 && FrameworkElementAutomationPeer.FromElement(PauseFailure) is { } peer)
            {
                peer.RaiseAutomationEvent(AutomationEvents.LiveRegionChanged);
            }
        }
    }

    /// <summary>
    /// The weekdays as toggle buttons, made once and then kept in step with the model (so keyboard
    /// focus stays on the one pressed). The last day not resting can't be made one.
    /// </summary>
    private void RenderRestDays()
    {
        var days = StatsFormat.Weekdays(Model.Culture);
        if (RestDays.Children.Count != days.Count)
        {
            RestDays.Children.Clear();
            foreach (var day in days)
            {
                var button = new ToggleButton { Content = day.Abbreviated, Tag = day, MinWidth = 0, Padding = new Thickness(12, 4, 12, 5) };
                button.Click += OnRestDayClicked;
                RestDays.Children.Add(button);
            }
        }
        foreach (var button in RestDays.Children.OfType<ToggleButton>())
        {
            if (button.Tag is not StatsFormat.Weekday day)
            {
                continue;
            }
            var rest = Model.RestDays.Contains(day.Iso);
            button.IsChecked = rest;
            button.IsEnabled = rest || Model.RestDays.Count < 6;
            // The name is the day; the toggle pattern says whether it is pressed.
            AutomationProperties.SetName(button, day.Name);
            // The group carries the sentence (RestDays' HelpText); each day only what is its own.
            AutomationProperties.SetHelpText(button, button.IsEnabled ? "" : StatsModel.LastRestDayHelp);
            ToolTipService.SetToolTip(button, button.IsEnabled ? day.Name : StatsModel.LastRestDayHelp);
        }
    }

    private void OnRestDayClicked(object sender, RoutedEventArgs e)
    {
        if (sender is ToggleButton { Tag: StatsFormat.Weekday day } button)
        {
            Model.SetRestDay(day.Iso, button.IsChecked == true);
            // Refused (the seventh), the button goes back to the model's.
            Render();
        }
    }

    private void OnStreakToggled(object sender, RoutedEventArgs e)
    {
        if (StreakShown.IsOn != Model.StreakShown)
        {
            Model.SetStreakShown(StreakShown.IsOn);
        }
    }

    private void OnPauseClicked(object sender, RoutedEventArgs e)
    {
        if (Model.Counted?.Dictation.StreakPausedSince is null)
        {
            Model.PauseStreak();
        }
        else
        {
            Model.ResumeStreak();
        }
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
