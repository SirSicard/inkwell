// Settings > AI. Each switch shows what the model says (on only with the core's consent and a
// working model), so switching one on reads back off while its consent step is up, and on once the
// core has recorded the consent. The section owns the Settings screen's ConsentDialog, for all
// three features (Dictation's edit-key picker asks through it too): WinUI shows one dialog at a time.
//
// Above them, the language model (LanguageModelRows, over CloudModel: the provider, its key, its
// model, Use and Test) and how to get a free Groq key (GroqKeyGuideView); then Local only, the
// explicit switch over llm.local_only.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class AiSection : UserControl
{
    private readonly AiSettings ai;
    private readonly PolishModel polish;
    private readonly CloudModel cloud;
    private bool rendering;

    public AiSection(AiSettings ai, CloudModel cloud, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(ai);
        ArgumentNullException.ThrowIfNull(cloud);
        this.ai = ai;
        this.cloud = cloud;
        polish = ai.Polish;
        InitializeComponent();
        LanguageModelHost.Content = new LanguageModelRows(cloud);
        GroqGuideExpander.Header = GroqKeyGuide.Title;
        GroqGuideExpander.Content = new GroqKeyGuideView(GroqKeyGuidePlace.Settings);
        _ = new ConsentDialog(this, ConsentHost.Settings, [polish.Consent, ai.EditConsent, ai.MeetingsConsent], log);
        Loaded += (_, _) =>
        {
            ai.PropertyChanged += OnChanged;
            cloud.PropertyChanged += OnChanged;
            // Read again whenever Settings appears, as the Mac's does: a read that failed is
            // retried here (its line says so), and a switch changed elsewhere is current.
            cloud.Load();
            polish.Load();
            ai.EditConsent.Load();
            ai.MeetingsConsent.Load();
            ai.Dictation.Load();
            Render();
        };
        Unloaded += (_, _) =>
        {
            ai.PropertyChanged -= OnChanged;
            cloud.PropertyChanged -= OnChanged;
        };
        Render();
    }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void OnLocalOnlyToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && LocalOnlySwitch.IsOn != cloud.LocalOnly)
        {
            cloud.SetLocalOnly(LocalOnlySwitch.IsOn);
        }
    }

    private void Render()
    {
        rendering = true;
        try
        {
            Show(PolishSwitch, PolishStatus, polish.IsOn, polish.CanToggle, polish.Status, polish.IsProblem);
            Show(EditSwitch, EditStatus, ai.EditOn, ai.CanToggleEdit, ai.EditStatus, ai.EditIsProblem);
            Show(MeetingsSwitch, MeetingsStatus, ai.MeetingsAIOn, ai.CanToggleMeetingsAI, ai.MeetingsAIStatus, ai.MeetingsAIIsProblem);
            RenderCloud();
            RenderConsents();
        }
        finally
        {
            rendering = false;
        }
    }

    private static void Show(ToggleSwitch toggle, TextBlock line, bool on, bool enabled, string status, bool problem)
    {
        toggle.IsOn = on;
        toggle.IsEnabled = enabled;
        AutomationProperties.SetHelpText(toggle, status);
        line.Text = status;
        line.Style = (Style)Application.Current.Resources[problem ? "InkAlertTextStyle" : "InkCaptionStyle"];
    }

    private void RenderCloud()
    {
        // Local only: on unless the user turned it off, or chose a provider off this PC.
        LocalOnlySwitch.IsOn = cloud.LocalOnly;
        LocalOnlySwitch.IsEnabled = cloud.Loaded;
        LocalOnlyCaption.Text = cloud.LocalOnly
            ? $"{CloudModel.LocalOnlyTitle}: no language model off this PC is called, whichever is chosen."
            : "A language model off this PC may be called, once a feature is on and allowed.";
    }

    /// <summary>The consents shown, so the list is built again only when they change (a Revoke keeps the keyboard otherwise).</summary>
    private IReadOnlyList<ConsentGrant>? shownConsents;

    /// <summary>Each destination the user agreed polish may send to, with Revoke.</summary>
    private void RenderConsents()
    {
        var consents = polish.State?.Consents ?? [];
        if (shownConsents is not null && shownConsents.SequenceEqual(consents))
        {
            return;
        }
        shownConsents = consents;
        PolishConsents.Children.Clear();
        PolishConsents.Children.Add(new TextBlock
        {
            Text = "Polish may send to",
            Style = (Style)Application.Current.Resources["InkBodyStyle"],
        });
        if (consents.Count == 0)
        {
            PolishConsents.Children.Add(Caption("Nowhere yet. Polish asks before it first sends anywhere."));
            return;
        }
        foreach (var grant in consents)
        {
            var row = new Grid { ColumnSpacing = 12 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var words = new StackPanel { Spacing = 1 };
            words.Children.Add(new TextBlock
            {
                Text = grant.Label,
                Style = (Style)Application.Current.Resources["InkBodyStyle"],
                TextWrapping = TextWrapping.Wrap,
            });
            words.Children.Add(Caption(grant.Detail));
            var revoke = new Button { Content = "Revoke", VerticalAlignment = VerticalAlignment.Center };
            AutomationProperties.SetName(revoke, $"Revoke polish's OK for {grant.Label}");
            revoke.Click += (_, _) => polish.Consent.Revoke(grant);
            Grid.SetColumn(revoke, 1);
            row.Children.Add(words);
            row.Children.Add(revoke);
            PolishConsents.Children.Add(row);
        }
        PolishConsents.Children.Add(Caption("Revoking the last one turns polish off. Modes on a model there go in as you said them."));
    }

    private static TextBlock Caption(string text) => new()
    {
        Text = text,
        Style = (Style)Application.Current.Resources["InkCaptionStyle"],
        TextWrapping = TextWrapping.Wrap,
    };

    private void OnPolishToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && PolishSwitch.IsOn != polish.IsOn)
        {
            polish.SetOn(PolishSwitch.IsOn);
            Render();
        }
    }

    private void OnEditToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && EditSwitch.IsOn != ai.EditOn)
        {
            ai.SetEdit(EditSwitch.IsOn);
            Render();
        }
    }

    private void OnMeetingsToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && MeetingsSwitch.IsOn != ai.MeetingsAIOn)
        {
            ai.SetMeetingsAI(MeetingsSwitch.IsOn);
            Render();
        }
    }
}
