// One permission card's row. It shows what PermissionCards says for the card's state; its button
// (only where Windows has something to ask) calls back to the cards view.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class PermissionRow : UserControl
{
    private readonly PermissionCard card;
    private readonly Action<PermissionCard> request;

    public PermissionRow(PermissionCard card, bool first, bool last, Action<PermissionCard> request)
    {
        ArgumentNullException.ThrowIfNull(request);
        this.card = card;
        this.request = request;
        InitializeComponent();
        TopRule.Visibility = first ? Visibility.Collapsed : Visibility.Visible;
        // The card's rounded corners (its border is 1 px): the red paper of an off row follows them.
        AlertFill.CornerRadius = new CornerRadius(first ? 13 : 0, first ? 13 : 0, last ? 13 : 0, last ? 13 : 0);
        TitleText.Text = card.Title();
        AutomationProperties.SetHelpText(ActionButton, card.Title());
    }

    /// <summary>Shows <paramref name="state"/>, and that its request failed when <paramref name="requestFailed"/>.</summary>
    public void Show(CardState state, bool requestFailed = false)
    {
        var off = state.IsAlert();
        AlertFill.Visibility = Visible(off);
        AllowedDot.Visibility = Visible(state == CardState.Allowed);
        AllowedCheck.Visibility = Visible(state == CardState.Allowed);
        OffRing.Visibility = Visible(off);
        PlainRing.Visibility = Visible(state is CardState.NotAsked or CardState.Unknown or CardState.Unavailable);
        // Spins only while a check is out: nothing animates once it answers.
        CheckingRing.IsActive = state == CardState.Checking;
        CheckingRing.Visibility = Visible(state == CardState.Checking);

        // A request that failed says so where the button is read, in the alert colour.
        var line = requestFailed ? PermissionCards.RequestFailedLine : card.Line(state);
        LineText.Text = line;
        OffLineText.Text = line;
        LineText.Style = (Style)Application.Current.Resources[requestFailed ? "InkAlertTextStyle" : "InkCaptionStyle"];
        LineText.Visibility = Visible(!off || requestFailed);
        OffLineText.Visibility = Visible(off && !requestFailed);

        if (card.ActionTitle(state) is string action)
        {
            ActionButton.Content = action;
            ActionButton.Style = off ? (Style)Application.Current.Resources["InkAccentButtonStyle"] : null;
            ActionButton.Visibility = Visibility.Visible;
            StateText.Visibility = Visibility.Collapsed;
        }
        else
        {
            ActionButton.Visibility = Visibility.Collapsed;
            StateText.Text = PermissionCards.StateLabel(state);
            StateText.Visibility = Visibility.Visible;
        }
        AutomationProperties.SetName(Root, requestFailed
            ? $"{card.Title()}: {card.Spoken(state)}. {PermissionCards.RequestFailedLine}"
            : $"{card.Title()}: {card.Spoken(state)}");
    }

    private void OnAction(object sender, RoutedEventArgs e) => request(card);

    private static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;
}
