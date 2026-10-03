// The catalogue's models where they can be downloaded. The rows and what each says are the
// CatalogueModel's (and, in the first run, OnboardingModel.ModelRows); this lays them out, and its
// buttons only ask the model. Redrawn on the model's changes while on screen: a download's progress
// comes about four times a second, and nothing redraws when none runs.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class ModelRowsView : UserControl
{
    private readonly CatalogueModel catalogue;
    private readonly bool firstRun;

    /// <param name="firstRun">The first run's rows (the models not installed), without a Download of their own.</param>
    public ModelRowsView(CatalogueModel catalogue, bool firstRun)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        this.catalogue = catalogue;
        this.firstRun = firstRun;
        InitializeComponent();
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
        Render();
    }

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
        var rows = firstRun ? OnboardingModel.ModelRows(catalogue) : catalogue.Rows;
        var focus = RowFocus.Capture(Rows, XamlRoot);
        // The first run's Download takes the recommended set; the optional models keep their own.
        Rows.ItemsSource = rows.Select(row => new ModelRowItem(row, OwnDownload: !firstRun || !CatalogueModel.IsRecommended(row.Id))).ToList();
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
}
