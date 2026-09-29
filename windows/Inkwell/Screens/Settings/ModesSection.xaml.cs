// Settings > Modes. Reads the modes each time the section appears; the rows are the model's.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class ModesSection : UserControl
{
    private IReadOnlyList<ModeRow>? shown;

    public ModesSection(ModesModel modes)
    {
        Model = modes ?? throw new ArgumentNullException(nameof(modes));
        InitializeComponent();
        Model.PropertyChanged += (_, _) => Render();
        Render();
    }

    public ModesModel Model { get; }

    public static string FailedText => ModesModel.FailedText;

    private void OnLoaded(object sender, RoutedEventArgs e) => Model.Load();

    private void Render()
    {
        if (ReferenceEquals(shown, Model.Rows))
        {
            return;
        }
        shown = Model.Rows;
        Rows.ItemsSource = Model.Rows.Select(row => new ModeRowItem(row, Model.Title(row))).ToList();
    }
}
