// The first-run sheet. Attach it once to an element of the window: it opens while
// OnboardingModel.Showing (once the element has a XamlRoot) and closes when that ends (Start, Skip,
// or the app quitting). Escape closes it as skipped (OnboardingModel.SheetDismissed, which does
// nothing while the app quits), except while polish's consent step is up in it: then Escape
// cancels the step and the sheet stays. The polish switch only asks (PolishModel.SetOn with
// ConsentHost.Onboarding); only the step's agreeing button sends anything.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Shapes;

namespace Inkwell.Screens;

public sealed partial class OnboardingSheet : ContentDialog
{
    private readonly FrameworkElement host;
    private readonly OnboardingModel onboarding;
    private readonly PermissionsModel permissions;
    private readonly PolishModel polish;
    private readonly DictationModel dictation;
    private readonly ScreenLog log;
    private bool isOpen;
    /// <summary>The sheet is closing because the model says so (or the host went): Escape's rule does not apply.</summary>
    private bool closingByModel;
    private bool rendering;

    private OnboardingSheet(
        FrameworkElement host, OnboardingModel onboarding, PermissionsModel permissions, PolishModel polish, DictationModel dictation, ScreenLog log)
    {
        this.host = host;
        this.onboarding = onboarding;
        this.permissions = permissions;
        this.polish = polish;
        this.dictation = dictation;
        this.log = log;
        InitializeComponent();
        CardsHost.Content = new PermissionCardsView(permissions);
        PermissionsTitle.Text = OnboardingModel.PermissionsTitle;
        PermissionsNote.Text = OnboardingModel.PermissionsNote;
        PolishTitle.Text = OnboardingModel.PolishTitle;
        PolishNote.Text = OnboardingModel.PolishNote;
        AutomationProperties.SetName(PolishSwitch, OnboardingModel.PolishToggle);
        ReadyTitle.Text = OnboardingModel.ReadyTitle;
        AutomationProperties.SetHelpText(SkipButton, OnboardingModel.SkipHint);
        ConsentTitle.Text = PolishModel.ConsentTitle;
        AutomationProperties.SetName(ConsentCancel, ConsentModel.CancelName(LlmFeature.Polish));
    }

    /// <summary>
    /// Shows the first run over <paramref name="host"/>'s window whenever the model says so. Call
    /// once, on the UI thread; the models are the ones the aggregator feeds.
    /// </summary>
    public static OnboardingSheet Attach(
        FrameworkElement host, OnboardingModel onboarding, PermissionsModel permissions, PolishModel polish, DictationModel dictation,
        ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(host);
        ArgumentNullException.ThrowIfNull(onboarding);
        ArgumentNullException.ThrowIfNull(permissions);
        ArgumentNullException.ThrowIfNull(polish);
        ArgumentNullException.ThrowIfNull(dictation);
        var sheet = new OnboardingSheet(host, onboarding, permissions, polish, dictation, log ?? ScreenLog.System);
        onboarding.PropertyChanged += (_, _) => sheet.Update();
        polish.PropertyChanged += (_, _) => sheet.RenderIfOpen();
        permissions.PropertyChanged += (_, _) => sheet.RenderIfOpen();
        dictation.PropertyChanged += (_, _) => sheet.RenderIfOpen();
        host.Loaded += (_, _) => sheet.Update();
        sheet.Update();
        return sheet;
    }

    private void Update()
    {
        if (onboarding.Showing && !isOpen && host.XamlRoot is not null)
        {
            Open();
        }
        else if (!onboarding.Showing && isOpen)
        {
            closingByModel = true;
            Hide();
        }
        else
        {
            RenderIfOpen();
        }
    }

    private async void Open()
    {
        isOpen = true;
        XamlRoot = host.XamlRoot;
        Render();
        try
        {
            await ShowAsync();
        }
        catch (Exception e) when (e is System.Runtime.InteropServices.COMException or InvalidOperationException)
        {
            // Another dialog is up: the first run shows at the next change of the model or the
            // next launch; nothing is recorded.
            log.Write("first-run sheet could not be shown");
            isOpen = false;
        }
    }

    private void RenderIfOpen()
    {
        if (isOpen)
        {
            Render();
        }
    }

    private void Render()
    {
        rendering = true;
        try
        {
            var step = onboarding.Step;
            WelcomeStep.Visibility = Visible(step == OnboardingStep.Welcome);
            PermissionsStep.Visibility = Visible(step == OnboardingStep.Permissions);
            PolishStep.Visibility = Visible(step == OnboardingStep.Polish);
            ReadyStep.Visibility = Visible(step == OnboardingStep.Ready);

            Ellipse[] dots = [Dot0, Dot1, Dot2, Dot3];
            for (var i = 0; i < dots.Length; i++)
            {
                dots[i].Opacity = i == (int)step ? 1 : 0.22;
            }
            AutomationProperties.SetName(Dots, onboarding.StepLabel);

            SkipButton.Visibility = Visible(onboarding.ShowsSkip);
            BackButton.Visibility = Visible(onboarding.ShowsBack);
            NextButton.Content = onboarding.NextTitle;

            var keyName = DictationModel.Key(dictation.CurrentKey)?.Name ?? DictationModel.Cap(dictation.CurrentKey);
            var welcome = OnboardingModel.WelcomeLines(keyName);
            WelcomeLine0.Text = welcome[0];
            WelcomeLine1.Text = welcome[1];
            WelcomeLine2.Text = welcome[2];

            PolishSwitch.IsOn = polish.IsOn;
            PolishSwitch.IsEnabled = polish.CanToggle;
            AutomationProperties.SetHelpText(PolishSwitch, polish.Status);
            PolishStatus.Text = polish.Status;
            PolishStatus.Style = (Style)Application.Current.Resources[polish.IsProblem ? "InkAlertTextStyle" : "InkCaptionStyle"];
            var asking = polish.Consent.IsShowingStep(ConsentHost.Onboarding) ? polish.PendingConsent : null;
            ConsentCard.Visibility = Visible(asking is not null);
            if (asking is not null)
            {
                ConsentMessage.Text = PolishModel.ConsentMessage(asking);
                ConsentAllow.Content = PolishModel.ConsentButton(asking);
                AutomationProperties.SetName(ConsentAllow, ConsentModel.AllowName(LlmFeature.Polish, asking));
            }

            ReadyLine.Text = OnboardingModel.ReadyLine(keyName);
            var stillOff = OnboardingModel.StillOff(permissions);
            StillOffLine.Text = stillOff ?? "";
            StillOffLine.Visibility = Visible(stillOff is not null);
        }
        finally
        {
            rendering = false;
        }
    }

    private void OnNext(object sender, RoutedEventArgs e)
    {
        onboarding.Next();
        Render();
    }

    private void OnBack(object sender, RoutedEventArgs e)
    {
        onboarding.Back();
        Render();
    }

    private void OnSkip(object sender, RoutedEventArgs e) => onboarding.Finish();

    private void OnPolishToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering)
        {
            polish.SetOn(PolishSwitch.IsOn, ConsentHost.Onboarding);
            Render();
        }
    }

    private void OnConsentCancel(object sender, RoutedEventArgs e) => polish.CancelConsent();

    private void OnConsentAllow(object sender, RoutedEventArgs e) => polish.AllowConsent();

    private void OnClosing(ContentDialog sender, ContentDialogClosingEventArgs args)
    {
        // Escape with polish's step up answers the step (Cancel), not the sheet.
        if (!closingByModel && onboarding.Showing && polish.Consent.IsShowingStep(ConsentHost.Onboarding))
        {
            args.Cancel = true;
            polish.CancelConsent();
        }
    }

    private void OnClosed(ContentDialog sender, ContentDialogClosedEventArgs args)
    {
        isOpen = false;
        closingByModel = false;
        if (polish.Consent.IsShowingStep(ConsentHost.Onboarding))
        {
            polish.CancelConsent();
        }
        // Escape: skipped, unless the app is quitting (the model knows).
        onboarding.SheetDismissed();
    }

    private static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;
}
