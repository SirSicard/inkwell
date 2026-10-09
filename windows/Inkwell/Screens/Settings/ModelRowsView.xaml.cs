// The catalogue's models where they can be downloaded (Settings > Models). The rows and what each
// says are the CatalogueModel's; this lays them out, and its buttons only ask the model (Remove
// after its question). Redrawn on the model's changes while on screen: a download's progress
// comes about four times a second, and nothing redraws when none runs.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace Inkwell.Screens;

public sealed partial class ModelRowsView : UserControl
{
    private readonly CatalogueModel catalogue;

    public ModelRowsView(CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        this.catalogue = catalogue;
        InitializeComponent();
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
        Render();
    }

    /// <summary>Secondary details keep the model's name and installed state easy to scan.</summary>
    public static string Metadata(ModelRow row) =>
        $"{StorageModel.Size(row.Entry.SizeBytes, System.Globalization.CultureInfo.CurrentCulture)} · {row.Entry.Licence}";

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        // Loaded can come twice without Unloaded between: one subscription only.
        catalogue.PropertyChanged -= OnChanged;
        catalogue.PropertyChanged += OnChanged;
        Render();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e) => catalogue.PropertyChanged -= OnChanged;

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void Render()
    {
        var focus = RowFocus.Capture(Rows, XamlRoot);
        Rows.ItemsSource = catalogue.Rows.Select(row => new ModelRowItem(row)).ToList();
        RowFocus.Restore(Rows, focus, item => ((ModelRowItem)item).Id);
    }

    private ModelRowItem? RowOf(object sender) =>
        RowTag.Of(sender) is string id ? (Rows.ItemsSource as IEnumerable<ModelRowItem>)?.FirstOrDefault(r => r.Id == id) : null;

    private void OnDownload(object sender, RoutedEventArgs e)
    {
        if (RowOf(sender) is ModelRowItem row)
        {
            catalogue.Download(row.Id);
        }
    }

    private void OnRetry(object sender, RoutedEventArgs e)
    {
        if (RowOf(sender) is ModelRowItem row)
        {
            catalogue.Download(row.Id);
        }
    }

    private void OnCancel(object sender, RoutedEventArgs e)
    {
        if (RowOf(sender) is ModelRowItem row)
        {
            catalogue.Cancel(row.Id);
        }
    }

    /// <summary>Remove asks first, saying what goes: Remove or Cancel, Cancel focused.</summary>
    private void OnRemove(object sender, RoutedEventArgs e)
    {
        if (RowOf(sender) is ModelRowItem item && sender is FrameworkElement anchor)
        {
            var id = item.Id;
            // Shown from this view, at the button: the rows are made again on every progress tick
            // while a download runs, and a flyout on a row's button would close with it.
            RemoveQuestion.Show(anchor, item.Row, () => catalogue.Remove(id), host: this);
        }
    }
}

/// <summary>
/// The question Remove asks before a model's files are deleted (Settings > Models and Settings >
/// AI's model on this PC): what goes and what stops, then Remove or Cancel, with Cancel focused so
/// Enter deletes nothing.
/// </summary>
internal static class RemoveQuestion
{
    /// <param name="host">A stable element to show it from, at <paramref name="anchor"/>'s place, when the anchor may be replaced while it is open; null: from the anchor.</param>
    public static void Show(FrameworkElement anchor, Inkwell.Core.Screens.ModelRow row, Action remove, FrameworkElement? host = null)
    {
        var question = new TextBlock { Text = row.RemoveQuestion, TextWrapping = TextWrapping.Wrap, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Style = (Style)Application.Current.Resources["InkBodyStyle"] };
        var detail = new TextBlock { Text = row.RemoveDetail(System.Globalization.CultureInfo.CurrentCulture), TextWrapping = TextWrapping.Wrap, Style = (Style)Application.Current.Resources["InkCaptionStyle"] };
        var confirm = new Button { Content = Inkwell.Core.Screens.ModelRow.RemoveConfirm, Style = (Style)Application.Current.Resources["InkAccentButtonStyle"] };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(confirm, row.RemoveName);
        var cancel = new Button { Content = "Cancel" };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(cancel, $"Cancel, and keep {row.Name}");
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        buttons.Children.Add(cancel);
        buttons.Children.Add(confirm);
        var panel = new StackPanel { Spacing = 8, Width = 320 };
        panel.Children.Add(question);
        panel.Children.Add(detail);
        panel.Children.Add(buttons);
        var flyout = new Flyout { Content = panel };
        confirm.Click += (_, _) =>
        {
            flyout.Hide();
            remove();
        };
        cancel.Click += (_, _) => flyout.Hide();
        flyout.Opened += (_, _) => cancel.Focus(FocusState.Programmatic);
        if (host is null)
        {
            flyout.ShowAt(anchor);
            return;
        }
        var at = anchor.TransformToVisual(host).TransformPoint(new Windows.Foundation.Point(0, anchor.ActualHeight));
        flyout.ShowAt(host, new FlyoutShowOptions { Position = at, Placement = FlyoutPlacementMode.BottomEdgeAlignedLeft });
    }
}
