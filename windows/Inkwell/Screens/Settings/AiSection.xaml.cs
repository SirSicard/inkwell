// Settings > AI. Each switch shows what the model says (on only with the core's consent and a
// working model), so switching one on reads back off while its consent step is up, and on once the
// core has recorded the consent. The section owns the Settings screen's ConsentDialog, for all
// three features (Dictation's edit-key picker asks through it too): WinUI shows one dialog at a time.
//
// Above them, the language model (CloudModel): the provider picker, the key (a PasswordBox whose
// text is sent once, on Save key, and cleared at once: it is never kept or shown), the model, Use
// and Test; then Local only, the explicit switch over llm.local_only. The pickers show what the core holds; a change is sent, and the core's answer is what
// shows.
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
    /// <summary>The provider ids behind the picker's items, in order (the first is none).</summary>
    private readonly List<string?> providerTokens = [];
    private bool rendering;

    public AiSection(AiSettings ai, CloudModel cloud, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(ai);
        ArgumentNullException.ThrowIfNull(cloud);
        this.ai = ai;
        this.cloud = cloud;
        polish = ai.Polish;
        InitializeComponent();
        ModelBox.RegisterPropertyChangedCallback(ComboBox.TextProperty, (_, _) => OnModelTyped());
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

        var items = new List<(string? Id, string Name)> { (null, "None: nothing leaves this PC") };
        items.AddRange(cloud.Providers.Select(p => ((string?)p.Id, p.Name)));
        if (!providerTokens.SequenceEqual(items.Select(i => i.Id)))
        {
            providerTokens.Clear();
            ProviderBox.Items.Clear();
            foreach (var (id, name) in items)
            {
                providerTokens.Add(id);
                ProviderBox.Items.Add(name);
            }
        }
        ProviderBox.SelectedIndex = providerTokens.IndexOf(cloud.Selected);
        ProviderBox.IsEnabled = cloud.Loaded;

        var provider = cloud.SelectedProvider;
        ProviderDetails.Visibility = Visible(provider is not null);
        ServerBox.Visibility = Visible(provider?.CustomUrl == true);
        if (ServerBox.Text != cloud.DraftBaseUrl)
        {
            ServerBox.Text = cloud.DraftBaseUrl;
        }
        // Short enough to show whole in the box; the line under it says a key is stored.
        KeyBox.PlaceholderText = provider?.HasKey == true ? "Paste a new key" : "Paste your API key";
        KeyStatus.Text = cloud.KeyStatus;
        DeleteKeyButton.IsEnabled = provider?.HasKey == true;
        DeleteKeyButton.Content = cloud.DeleteKeyLabel;
        var models = new List<string>();
        if (provider is not null)
        {
            models.Add(provider.DefaultModel);
            if (cloud.Chosen == provider.Id && cloud.ChosenModel is string chosen && chosen != provider.DefaultModel)
            {
                models.Add(chosen);
            }
        }
        if (!models.SequenceEqual(ModelBox.Items.OfType<string>()))
        {
            ModelBox.Items.Clear();
            foreach (var model in models)
            {
                ModelBox.Items.Add(model);
            }
        }
        ModelBox.PlaceholderText = provider?.DefaultModel ?? "";
        if (ModelBox.Text != cloud.DraftModel)
        {
            ModelBox.Text = cloud.DraftModel;
        }

        UseNote.Text = cloud.UseNote;
        UseButton.Content = cloud.UseLabel;
        UseButton.IsEnabled = cloud.CanUse;
        AutomationProperties.SetHelpText(UseButton, cloud.UseNote);
        TestButton.IsEnabled = cloud.CanTest;
        Line(TestStatus, cloud.TestMessage, cloud.TestState == CloudTestState.Failed);
        var status = cloud.Failure ?? cloud.Status;
        Line(CloudStatus, status, cloud.Failure is not null || cloud.ReadError is not null);
        AutomationProperties.SetHelpText(ProviderBox, status);
    }

    private static void Line(TextBlock line, string? text, bool problem)
    {
        line.Text = text ?? "";
        line.Visibility = Visible(!string.IsNullOrEmpty(text));
        line.Style = (Style)Application.Current.Resources[problem ? "InkAlertTextStyle" : "InkCaptionStyle"];
    }

    private static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;

    private void OnProviderChosen(object sender, SelectionChangedEventArgs e)
    {
        var i = ProviderBox.SelectedIndex;
        if (!rendering && i >= 0 && i < providerTokens.Count && providerTokens[i] != cloud.Selected)
        {
            KeyBox.Password = "";
            cloud.Select(providerTokens[i]);
        }
    }

    private void OnServerChanged(object sender, TextChangedEventArgs e)
    {
        if (!rendering && ServerBox.Text != cloud.DraftBaseUrl)
        {
            cloud.DraftBaseUrl = ServerBox.Text;
            Render();
        }
    }

    private void OnModelChosen(object sender, SelectionChangedEventArgs e)
    {
        if (!rendering && ModelBox.SelectedItem is string model)
        {
            cloud.DraftModel = model;
            Render();
        }
    }

    private void OnModelTyped()
    {
        if (!rendering && ModelBox.Text != cloud.DraftModel)
        {
            cloud.DraftModel = ModelBox.Text ?? "";
            Render();
        }
    }

    private void OnSaveKey(object sender, RoutedEventArgs e)
    {
        // Sent once, then gone from the box: the key is never kept or shown here.
        var key = KeyBox.Password;
        KeyBox.Password = "";
        cloud.SaveKey(key);
    }

    /// <summary>Delete asks first, saying which key goes and that every Inkwell on the account loses it.</summary>
    private void OnDeleteKey(object sender, RoutedEventArgs e)
    {
        var question = new TextBlock { Text = cloud.DeleteKeyQuestion, TextWrapping = TextWrapping.Wrap, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Style = (Style)Application.Current.Resources["InkBodyStyle"] };
        var detail = new TextBlock { Text = CloudModel.DeleteKeyDetail, TextWrapping = TextWrapping.Wrap, Style = (Style)Application.Current.Resources["InkCaptionStyle"] };
        var delete = new Button { Content = cloud.DeleteKeyLabel, Style = (Style)Application.Current.Resources["InkAccentButtonStyle"] };
        var cancel = new Button { Content = "Cancel" };
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        buttons.Children.Add(cancel);
        buttons.Children.Add(delete);
        var panel = new StackPanel { Spacing = 8, Width = 320 };
        panel.Children.Add(question);
        panel.Children.Add(detail);
        panel.Children.Add(buttons);
        var flyout = new Flyout { Content = panel };
        delete.Click += (_, _) =>
        {
            flyout.Hide();
            cloud.DeleteKey();
        };
        cancel.Click += (_, _) => flyout.Hide();
        // Cancel first: Enter on an opened question deletes nothing.
        flyout.Opened += (_, _) => cancel.Focus(FocusState.Programmatic);
        flyout.ShowAt(DeleteKeyButton);
    }

    private void OnUse(object sender, RoutedEventArgs e) => cloud.Use();

    private void OnTest(object sender, RoutedEventArgs e) => cloud.Test();

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
