// The four permission cards. While on screen they are checked when they appear and again each
// time the app becomes active (the model's rule; the window calls PermissionsModel.AppBecameActive),
// and never on a timer. A card's button asks through the model: the microphone's opens Windows
// Settings (the core does), the others have nothing to ask on Windows and show no button.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class PermissionCardsView : UserControl
{
    private readonly PermissionsModel permissions;
    private readonly Dictionary<PermissionCard, PermissionRow> rows = [];

    /// <param name="inSettings">Settings > Permissions: inside the section's card, so flat (InkInsetCardStyle).</param>
    public PermissionCardsView(PermissionsModel permissions, bool inSettings)
    {
        ArgumentNullException.ThrowIfNull(permissions);
        this.permissions = permissions;
        InitializeComponent();
        if (inSettings)
        {
            Frame.Style = (Style)Application.Current.Resources["InkInsetCardStyle"];
            Frame.Padding = new Thickness(0);
        }
        var all = PermissionCards.All;
        for (var i = 0; i < all.Count; i++)
        {
            var row = new PermissionRow(all[i], first: i == 0, last: i == all.Count - 1, permissions.Request);
            rows[all[i]] = row;
            Rows.Children.Add(row);
        }
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
        Render();
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        permissions.PropertyChanged += OnChanged;
        permissions.ScreenAppeared();
        Render();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        permissions.PropertyChanged -= OnChanged;
        permissions.ScreenDisappeared();
    }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void Render()
    {
        foreach (var (card, row) in rows)
        {
            row.Show(permissions.State(card), permissions.RequestFailed == card);
        }
    }
}
