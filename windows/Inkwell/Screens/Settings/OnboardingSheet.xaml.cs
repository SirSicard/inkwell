// The first-run sheet. Attach it once to an element of the window: it opens while
// OnboardingModel.Showing (once the element has a XamlRoot) and closes when that ends (Start, Skip,
// or the app quitting). Escape closes it as skipped (OnboardingModel.SheetDismissed, which does
// nothing while the app quits), except while polish's consent step is up in it: then Escape
// cancels the step and the sheet stays. The polish switch only asks (PolishModel.SetOn with
// ConsentHost.Onboarding); only the step's agreeing button sends anything. The models step's
// Download is the only thing in the sheet that downloads (CatalogueModel.DownloadMissing). The
// import step shows only while Inkwell 0.2's data is offered (OnboardingModel.ShownSteps). While
// this PC has no language model, the Polish step offers Groq's free key through Settings > AI's
// flow (CloudModel.UseKey); its box is sent once and cleared, as there.
//
// Glow's steps: Welcome's orb plays a short demo (dictating, then a call: a one-shot timer per
// part, only while Welcome shows); Appearance sets the mode and the dots of the mode shown, as
// Settings does; Ready's orb follows the shell's ink, so it answers the voice when the user holds
// the key, and the words land in its box.
using System.Globalization;
using Inkwell.Core.Events;
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Inkwell.Ink;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;

namespace Inkwell.Screens;

public sealed partial class OnboardingSheet : ContentDialog
{
    private readonly FrameworkElement host;
    private readonly OnboardingModel onboarding;
    private readonly PermissionsModel permissions;
    private readonly PolishModel polish;
    private readonly CloudModel cloud;
    private readonly DictationModel dictation;
    private readonly CatalogueModel catalogue;
    private readonly Import02Model import02;
    private readonly GlowTheme theme;
    private readonly ShellInk? ink;
    private readonly ScreenLog log;
    private readonly Dictionary<string, ToggleButton> presets = [];
    private readonly DispatcherQueueTimer demo;
    private bool isOpen;
    /// <summary>The sheet is closing because the model says so (or the host went): Escape's rule does not apply.</summary>
    private bool closingByModel;
    private bool rendering;

    private OnboardingSheet(
        FrameworkElement host, OnboardingModel onboarding, PermissionsModel permissions, PolishModel polish, CloudModel cloud,
        DictationModel dictation, CatalogueModel catalogue, Import02Model import02, ImportNoteModel importNote, GlowTheme theme,
        ShellInk? ink, ScreenLog log)
    {
        this.theme = theme;
        this.ink = ink;
        this.host = host;
        this.onboarding = onboarding;
        this.permissions = permissions;
        this.polish = polish;
        this.cloud = cloud;
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
        OwnKeyLine.Text = OnboardingModel.OwnKeyLine;
        OwnKeyLink.NavigateUri = new Uri(OnboardingModel.OwnKeyUrl);
        AutomationProperties.SetName(OwnKeyBox, OnboardingModel.OwnKeyBoxName);
        OwnKeyBox.PlaceholderText = OnboardingModel.OwnKeyBoxName;
        OwnKeyUse.Content = OnboardingModel.OwnKeyButton;
        AutomationProperties.SetHelpText(OwnKeyUse, OnboardingModel.OwnKeyNote);
        OwnKeyNote.Text = OnboardingModel.OwnKeyNote;
        ReadyTitle.Text = OnboardingModel.ReadyTitle;
        AutomationProperties.SetHelpText(SkipButton, OnboardingModel.SkipHint);
        ConsentTitle.Text = PolishModel.ConsentTitle;
        AutomationProperties.SetName(ConsentCancel, ConsentModel.CancelName(LlmFeature.Polish));
        AppearanceTitle.Text = OnboardingModel.AppearanceTitle;
        AppearanceNote.Text = OnboardingModel.AppearanceNote;
        foreach (var preset in GlowScheme.Presets)
        {
            var button = PresetButton(preset);
            presets[preset.Id] = button;
            PresetRow.Children.Add(button);
        }
        // The first run's orbs sit in the middle of their boxes, as the Drop's does.
        var middle = new InkPlacement(GlowTokens.Orb.Drop.X, GlowTokens.Orb.Drop.Y, GlowTokens.Orb.Drop.Unit);
        WelcomeOrb.Placement = middle;
        WelcomeOrb.Demo = true;
        ReadyOrb.Placement = middle;
        demo = DispatcherQueue.GetForCurrentThread().CreateTimer();
        demo.IsRepeating = false;
        demo.Tick += (_, _) => DemoNext();
        theme.Changed += () =>
        {
            RequestedTheme = host.ActualTheme;
            ShowLook();
            RenderIfOpen();
        };
        ShowLook();
        if (ink is not null)
        {
            ink.Changed += () => ReadyOrb.State = ink.State;
        }
    }

    /// <summary>A preset's button: its two dots, named for Narrator and the tooltip.</summary>
    private ToggleButton PresetButton(GlowPreset preset)
    {
        var dots = new Grid { Width = 34, Height = 20 };
        dots.Children.Add(new Ellipse { Width = 20, Height = 20, HorizontalAlignment = HorizontalAlignment.Left, Fill = new SolidColorBrush(GlowTheme.ColorOf(GlowRgb.From(preset.You))) });
        dots.Children.Add(new Ellipse { Width = 20, Height = 20, HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(14, 0, 0, 0), Opacity = 0.9, Fill = new SolidColorBrush(GlowTheme.ColorOf(GlowRgb.From(preset.Them))) });
        var button = new ToggleButton { Content = dots, Padding = new Thickness(10, 8, 10, 8), CornerRadius = new CornerRadius(999) };
        AutomationProperties.SetName(button, preset.Name);
        ToolTipService.SetToolTip(button, preset.Name);
        button.Click += (_, _) =>
        {
            if (!rendering)
            {
                theme.Appearance.SetDots(theme.Dark, preset.Id);
            }
        };
        return button;
    }

    /// <summary>The orbs in the colours shown.</summary>
    private void ShowLook()
    {
        WelcomeOrb.Look = theme.Look;
        WelcomeOrb.AlwaysStill = theme.AlwaysStill;
        ReadyOrb.Look = theme.Look;
        ReadyOrb.AlwaysStill = theme.AlwaysStill;
        ReadyOrb.State = ink?.State ?? InkState.Idle;
    }

    /// <summary>Welcome's demo: dictating for six seconds, then a call for eight, again, while Welcome shows.</summary>
    private void DemoNext()
    {
        var showing = isOpen && onboarding.Step == OnboardingStep.Welcome;
        if (!showing)
        {
            demo.Stop();
            WelcomeOrb.State = InkState.Idle;
            return;
        }
        var dictating = WelcomeOrb.State != InkState.Dictating;
        WelcomeOrb.State = dictating ? InkState.Dictating : InkState.Meeting;
        demo.Interval = TimeSpan.FromSeconds(dictating ? 6 : 8);
        demo.Start();
    }

    /// <summary>
    /// Shows the first run over <paramref name="host"/>'s window whenever the model says so. Call
    /// once, on the UI thread; the models are the ones the aggregator feeds.
    /// </summary>
    internal static OnboardingSheet Attach(
        FrameworkElement host, OnboardingModel onboarding, PermissionsModel permissions, PolishModel polish, CloudModel cloud,
        DictationModel dictation, CatalogueModel catalogue, Import02Model import02, ImportNoteModel importNote, GlowTheme theme,
        ShellInk? ink, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(theme);
        ArgumentNullException.ThrowIfNull(host);
        ArgumentNullException.ThrowIfNull(onboarding);
        ArgumentNullException.ThrowIfNull(permissions);
        ArgumentNullException.ThrowIfNull(polish);
        ArgumentNullException.ThrowIfNull(cloud);
        ArgumentNullException.ThrowIfNull(dictation);
        ArgumentNullException.ThrowIfNull(catalogue);
        ArgumentNullException.ThrowIfNull(import02);
        ArgumentNullException.ThrowIfNull(importNote);
        var sheet = new OnboardingSheet(
            host, onboarding, permissions, polish, cloud, dictation, catalogue, import02, importNote, theme, ink, log ?? ScreenLog.System);
        onboarding.PropertyChanged += (_, _) => sheet.Update();
        polish.PropertyChanged += (_, _) => sheet.RenderIfOpen();
        // The own key's answer: stored and chosen (polish then has a model), or why not.
        cloud.PropertyChanged += (_, _) => sheet.RenderIfOpen();
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
        // A dialog does not take the window's theme: it follows the appearance itself.
        RequestedTheme = host.ActualTheme;
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
            AppearanceStep.Visibility = Visible(step == OnboardingStep.Appearance);
            PolishStep.Visibility = Visible(step == OnboardingStep.Polish);
            ReadyStep.Visibility = Visible(step == OnboardingStep.Ready);
            // The demo plays only while Welcome shows.
            if (step == OnboardingStep.Welcome && !demo.IsRunning && WelcomeOrb.State == InkState.Idle)
            {
                DemoNext();
            }
            else if (step != OnboardingStep.Welcome)
            {
                demo.Stop();
                WelcomeOrb.State = InkState.Idle;
            }

            ModeChoice.SelectedIndex = theme.Appearance.Mode switch
            {
                AppearanceMode.Light => 0,
                AppearanceMode.Dark => 1,
                _ => 2,
            };
            var chosen = theme.Appearance.Dots(theme.Dark);
            foreach (var (id, button) in presets)
            {
                button.IsChecked = id == chosen;
            }

            // One dot per step shown.
            var shown = onboarding.ShownSteps.ToList();
            Ellipse[] dots = [Dot0, Dot1, Dot2, Dot3, Dot4, Dot5, Dot6];
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
            var meetingNote = OnboardingModel.MeetingModelNote(catalogue);
            MeetingModelNote.Text = meetingNote ?? "";
            MeetingModelNote.Visibility = Visible(meetingNote is not null);
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
            // Offered only while there is no language model; once one is ready, the switch can be used.
            OwnKeyPanel.Visibility = Visible(!polish.HasWorkingEngine);
            OwnKeyUse.IsEnabled = cloud.Loaded;
            OwnKeyStatus.Text = cloud.Failure ?? "";
            OwnKeyStatus.Visibility = Visible(cloud.Failure is not null);
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
    private void OnDownload(object sender, RoutedEventArgs e) => catalogue.DownloadRecommended();

    private void OnModelsTryAgain(object sender, RoutedEventArgs e) => catalogue.Requery();

    private void OnPolishToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && PolishSwitch.IsOn != polish.IsOn)
        {
            polish.SetOn(PolishSwitch.IsOn, ConsentHost.Onboarding);
            Render();
        }
    }

    private void OnOwnKeyUse(object sender, RoutedEventArgs e)
    {
        // Sent once, then gone from the box: the key is never kept or shown here (as in Settings > AI).
        var key = OwnKeyBox.Password;
        OwnKeyBox.Password = "";
        cloud.UseKey(OnboardingModel.OwnKeyProvider, key);
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

    private void OnModeChosen(object sender, SelectionChangedEventArgs e)
    {
        if (!rendering && ModeChoice.SelectedIndex is >= 0 and < 3)
        {
            theme.Appearance.SetMode(ModeChoice.SelectedIndex switch
            {
                0 => AppearanceMode.Light,
                1 => AppearanceMode.Dark,
                _ => AppearanceMode.System,
            });
        }
    }

    private void OnClosed(ContentDialog sender, ContentDialogClosedEventArgs args)
    {
        isOpen = false;
        demo.Stop();
        WelcomeOrb.State = InkState.Idle;
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
