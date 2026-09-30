using Inkwell.Core.Screens;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class PermissionsSection : UserControl
{
    public PermissionsSection(PermissionsModel permissions)
    {
        InitializeComponent();
        CardsHost.Content = new PermissionCardsView(permissions);
    }
}
