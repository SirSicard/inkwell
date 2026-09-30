// Inkwell 0.2's history, as Import02Model says it. It never looks for the data itself: the first
// run looks once (ScreenModels), and Settings looks each time its Voice section loads (Screens).
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class Import02Card : UserControl
{
    private readonly Import02Model model;
    private readonly bool inSettings;

    /// <param name="inSettings">Settings > Voice: a card, shown only while there is something to say.
    /// Otherwise the first run's step, which shows only then.</param>
    public Import02Card(Import02Model import02, bool inSettings)
    {
        model = import02 ?? throw new ArgumentNullException(nameof(import02));
        this.inSettings = inSettings;
        InitializeComponent();
        ImportButton.Content = Import02Model.ImportButton;
        AutomationProperties.SetHelpText(ImportButton, Import02Model.ImportHint);
        if (inSettings)
        {
            Frame.Style = (Style)Application.Current.Resources["InkCardStyle"];
            Frame.Padding = new Thickness(12);
        }
        Loaded += (_, _) =>
        {
            model.PropertyChanged += OnChanged;
            Render();
        };
        Unloaded += (_, _) => model.PropertyChanged -= OnChanged;
        Render();
    }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void Render()
    {
        Visibility = Visible(!inSettings || model.ShownInSettings);
        var line = model.Line;
        LineText.Text = line;
        LineText.Style = (Style)Application.Current.Resources[model.LineIsProblem ? "InkAlertTextStyle" : "InkBodyStyle"];
        LineText.Visibility = Visible(line.Length > 0);
        ImportButton.Visibility = Visible(model.CanImport);
        FailureText.Text = model.Failure ?? "";
        FailureText.Visibility = Visible(model.Failure is not null);
    }

    private void OnImport(object sender, RoutedEventArgs e) => model.Run();

    private static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;
}
