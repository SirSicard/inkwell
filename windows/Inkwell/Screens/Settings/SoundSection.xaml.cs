// Settings > Sound. The pickers and the meter are the model's (SoundModel); a refused choice or a
// list that could not be read is said under its picker. The devices are read each time the section
// shows (Load), and follow the core's audio.devices_changed after that; leaving stops a running test.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class SoundSection : UserControl
{
    /// <summary>The pickers are being filled from the model, not chosen by the user.</summary>
    private bool rendering;
    private IReadOnlyList<SoundModel.Choice> inputs = [];
    private IReadOnlyList<SoundModel.Choice> outputs = [];

    public SoundSection(SoundModel sound)
    {
        Model = sound ?? throw new ArgumentNullException(nameof(sound));
        InitializeComponent();
        Render();
        Loaded += (_, _) =>
        {
            Model.PropertyChanged += OnChanged;
            Model.Load();
        };
        Unloaded += (_, _) =>
        {
            Model.PropertyChanged -= OnChanged;
            Model.Disappeared();
        };
    }

    public SoundModel Model { get; }

    /// <summary>The test state last drawn: a level alone only moves the meter.</summary>
    private (SoundModel.TestState, SoundModel.Devices?, string?, string?)? drawn;

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void Render()
    {
        Meter.Value = Model.TestLevel;
        var now = (Model.Test, Model.Current, Model.InputProblem ?? Model.OutputProblem, Model.TestRefused);
        if (drawn == now)
        {
            // A level: the bar moves, and nothing else is read again (the meter's value is its own).
            return;
        }
        drawn = now;
        rendering = true;
        try
        {
            inputs = Fill(InputPicker, Model.InputChoices, Model.Current?.Input);
            outputs = Fill(OutputPicker, Model.OutputChoices, Model.Current?.Output);
        }
        finally
        {
            rendering = false;
        }
        InputPicker.IsEnabled = Model.Current is not null;
        AutomationProperties.SetHelpText(InputPicker, Model.InputCaption);
        // The caption says the missing mic itself; the line under it is what Narrator reads as it happens.
        InputCaption.Text = Model.MissingLine is null ? Model.InputCaption : "";
        InputCaption.Visibility = Model.MissingLine is null ? Visibility.Visible : Visibility.Collapsed;
        Show(MissingLine, Model.MissingLine);
        Show(InputProblem, Model.InputProblem);
        OutputRow.Visibility = Model.HasOutputs ? Visibility.Visible : Visibility.Collapsed;
        AutomationProperties.SetHelpText(OutputPicker, Model.OutputCaption);
        OutputCaption.Text = Model.OutputCaption;
        Show(OutputProblem, Model.OutputProblem);

        TestButton.Content = Model.IsTesting ? "Stop" : "Test";
        AutomationProperties.SetName(TestButton, Model.IsTesting ? "Stop the microphone test" : "Test the microphone");
        // Stop always works: the last mic may go mid-test.
        TestButton.IsEnabled = Model.IsTesting || Model.Current is not { Inputs.Count: 0 };
        // The bar's own value is its level for Narrator; the status says only whether a test runs.
        AutomationProperties.SetItemStatus(Meter, Model.Test == SoundModel.TestState.Running ? "Testing" : "No test running");
        Show(TestLine, Model.TestLineIsProblem ? null : Model.TestLine);
        Show(TestProblem, Model.TestLineIsProblem ? Model.TestLine : null);
    }

    /// <summary>Puts <paramref name="choices"/> in <paramref name="picker"/>, the one with <paramref name="selected"/> chosen.</summary>
    private static IReadOnlyList<SoundModel.Choice> Fill(ComboBox picker, IReadOnlyList<SoundModel.Choice> choices, string? selected)
    {
        var titles = choices.Select(c => c.Title).ToList();
        if (!picker.Items.Cast<object>().Select(o => o as string).SequenceEqual(titles))
        {
            picker.Items.Clear();
            foreach (var title in titles)
            {
                picker.Items.Add(title);
            }
        }
        var index = choices.ToList().FindIndex(c => c.Id == selected);
        if (picker.SelectedIndex != index)
        {
            picker.SelectedIndex = index;
        }
        return choices;
    }

    private static void Show(TextBlock line, string? text)
    {
        line.Text = text ?? "";
        line.Visibility = text is null ? Visibility.Collapsed : Visibility.Visible;
    }

    private void OnInputChosen(object sender, SelectionChangedEventArgs e)
    {
        if (!rendering && InputPicker.SelectedIndex is var i && i >= 0 && i < inputs.Count)
        {
            Model.ChooseInput(inputs[i].Id);
        }
    }

    private void OnOutputChosen(object sender, SelectionChangedEventArgs e)
    {
        if (!rendering && OutputPicker.SelectedIndex is var i && i >= 0 && i < outputs.Count)
        {
            Model.ChooseOutput(outputs[i].Id);
        }
    }

    private void OnTest(object sender, RoutedEventArgs e) => Model.ToggleTest();
}
