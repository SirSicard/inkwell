// The first-run sheet. Attach it once to an element of the window: it opens while
// OnboardingModel.Showing (once the element has a XamlRoot) and closes when that ends (Start, Skip,
// or the app quitting). Escape closes it as skipped (OnboardingModel.SheetDismissed, which does
// nothing while the app quits), except while polish's consent step is up in it: then Escape
// cancels the step and the sheet stays. The polish switch only asks (PolishModel.SetOn with
// ConsentHost.Onboarding); only the step's agreeing button sends anything. The models step's
// Download is the only thing in the sheet that downloads (CatalogueModel.DownloadMissing). The
// import step shows only while Inkwell 0.2's data is offered (OnboardingModel.ShownSteps).
using System.Globalization;
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
    private readonly CatalogueModel catalogue;
    private readonly Import02Model import02;
    private readonly ScreenLog log;
    private bool isOpen;
    /// <summary>The sheet is closing because the model says so (or the host went): Escape's rule does not apply.</summary>
    private bool closingByModel;
    private bool rendering;

    private OnboardingSheet(
        FrameworkElement host, OnboardingModel onboarding, PermissionsModel permissions, PolishModel polish, DictationModel dictation,
        CatalogueModel catalogue, Import02Model import02, ImportNoteModel importNote, ScreenLog log)
    {
        this.host = host;
        this.onboarding = onboarding;
        this.permissions = permissions;
        this.polish = polish;
        this.dictation = dictation;
        this.catalogue = catalogue;
        this.import02 = import02;
        this.log = log;
        InitializeComponent();
        CardsHost.Content = new PermissionCardsView(permissions);
        ImportTitle.Text = Import02Model.StepTitle;
        ImportCardHost.Content = new Import02Card(import02, inSettings: false);
        ImportNoteHost.Content = new ImportKeyNoteView(importNote, KeyName);
        PermissionsTitle.Text = OnboardingModel.PermissionsTitle;
        PermissionsNote.Text = OnboardingModel.PermissionsNote;
        ModelRowsHost.Content = new ModelRowsView(catalogue, firstRun: true);
        ModelsTitle.Text = OnboardingModel.ModelsTitle;
        DownloadButton.Content = OnboardingModel.DownloadTitle;
        ModelsTryAgain.Content = OnboardingModel.ModelsTryAgain;
        ModelsGoOn.Text = OnboardingModel.ModelsGoOn;
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
        CatalogueModel catalogue, Import02Model import02, ImportNoteModel importNote, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(host);
        ArgumentNullException.ThrowIfNull(onboarding);
        ArgumentNullException.ThrowIfNull(permissions);
        ArgumentNullException.ThrowIfNull(polish);
        ArgumentNullException.ThrowIfNull(dictation);
        ArgumentNullException.ThrowIfNull(catalogue);
        ArgumentNullException.ThrowIfNull(import02);
        ArgumentNullException.ThrowIfNull(importNote);
        var sheet = new OnboardingSheet(host, onboarding, permissions, polish, dictation, catalogue, import02, importNote, log ?? ScreenLog.System);
        onboarding.PropertyChanged += (_, _) => sheet.Update();
        polish.PropertyChanged += (_, _) => sheet.RenderIfOpen();
        permissions.PropertyChanged += (_, _) => sheet.RenderIfOpen();
        dictation.PropertyChanged += (_, _) => sheet.RenderIfOpen();
        // The models step's lines: the list, what is left to ask for, whether a download runs.
        catalogue.PropertyChanged += (_, _) => sheet.RenderIfOpen();
        // Whether the import step shows, and Not now or Continue on it.
        import02.PropertyChanged += (_, _) => sheet.RenderIfOpen();
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
        catch (Exception e)
        {
            // Another dialog is up (or the dialog failed): the first run shows at the next change
            // of the model or the next launch; nothing is recorded.
            log.Write($"first-run sheet could not be shown ({e.GetType().Name})");
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
            ModelsStep.Visibility = Visible(step == OnboardingStep.Models);
            ImportStep.Visibility = Visible(step == OnboardingStep.ImportData);
            ImportNoteHost.Visibility = Visible(import02.Imported is not null);
            PolishStep.Visibility = Visible(step == OnboardingStep.Polish);
            ReadyStep.Visibility = Visible(step == OnboardingStep.Ready);

            // One dot per step shown.
            var shown = onboarding.ShownSteps.ToList();
            Ellipse[] dots = [Dot0, Dot1, Dot2, Dot3, Dot4, Dot5];
            for (var i = 0; i < dots.Length; i++)
            {
                dots[i].Visibility = Visible(i < shown.Count);
                dots[i].Opacity = i == shown.IndexOf(step) ? 1 : 0.22;
            }
            AutomationProperties.SetName(Dots, onboarding.StepLabel);

            SkipButton.Visibility = Visible(onboarding.ShowsSkip);
            BackButton.Visibility = Visible(onboarding.ShowsBack);
            NextButton.Content = onboarding.NextTitle;

            var keyName = KeyName();
            var welcome = OnboardingModel.WelcomeLines(keyName);
            WelcomeLine0.Text = welcome[0];
            WelcomeLine1.Text = welcome[1];
            WelcomeLine2.Text = welcome[2];

            ModelsNote.Text = OnboardingModel.ModelsNote(catalogue);
            var downloadLine = OnboardingModel.DownloadLine(catalogue, CultureInfo.CurrentCulture);
            DownloadLine.Text = downloadLine ?? "";
            DownloadLine.Visibility = Visible(downloadLine is not null);
            DownloadButton.Visibility = Visible(downloadLine is not null);
            if (downloadLine is not null)
            {
                AutomationProperties.SetName(DownloadButton, OnboardingModel.DownloadName(catalogue, CultureInfo.CurrentCulture));
            }
            ModelsTryAgain.Visibility = Visible(catalogue.Failed);
            ModelsGoOn.Visibility = Visible(catalogue.Downloading);

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

    /// <summary>The user's agreement to the models the step names: the only download the sheet starts.</summary>
    private void OnDownload(object sender, RoutedEventArgs e) => catalogue.DownloadMissing();

    private void OnModelsTryAgain(object sender, RoutedEventArgs e) => catalogue.Requery();

    private void OnPolishToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && PolishSwitch.IsOn != polish.IsOn)
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

    /// <summary>The dictation key's name now.</summary>
    private string KeyName() => DictationModel.Key(dictation.CurrentKey)?.Name ?? DictationModel.Cap(dictation.CurrentKey);

    private static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;
}
