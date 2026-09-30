// Settings > Models. The lines and the downloadable rows (ModelRowsView) are the model's; this lays
// them out. Asks the core again each time the section appears (engine.routed answers only
// engine.route).
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class ModelsSection : UserControl
{
    public ModelsSection(CatalogueModel catalogue)
    {
        Model = catalogue ?? throw new ArgumentNullException(nameof(catalogue));
        InitializeComponent();
        DownloadableHost.Content = new ModelRowsView(Model, firstRun: false);
        Model.PropertyChanged += (_, _) => Render();
        Render();
    }

    public CatalogueModel Model { get; }

    public static string FailedText => CatalogueModel.FailedText;

    public static string SourceText => CatalogueModel.SourceText;

    private void OnLoaded(object sender, RoutedEventArgs e) => Model.Requery();

    private void Render()
    {
        Lines.ItemsSource = CatalogueModel.Jobs.Select(job => CatalogueLineItem.Of(Model, job)).ToList();
        DownloadablePanel.Visibility = Model.Models.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    }
}
