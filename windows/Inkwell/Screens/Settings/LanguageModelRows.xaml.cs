// The language model's rows (CloudModel): the provider picker, the key (a PasswordBox whose text
// is sent once, on Save key, and cleared at once: it is never kept or shown), the model, Use and
// Test. The pickers show what the core holds; a change is sent, and the core's answer is what
// shows. In the first run (firstRun: polish) Use asks polish's consent before it chooses
// (PolishModel.UseOwnKey), so local-only mode goes off only with the user's agreement, and only the
// own-key providers are offered (the Polish step offers this PC's model itself). In Settings
// (local) the picker holds this PC's own model too: its download's rows, then Use, which chooses
// it and asks polish's one tap (LocalLlmModel), and Try it (llm.test, timed).
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class LanguageModelRows : UserControl
{
    private readonly CloudModel cloud;
    private readonly PolishModel? firstRun;
    private readonly LocalLlmModel? local;
    /// <summary>The provider ids behind the picker's items, in order (the first is none).</summary>
    private readonly List<string?> providerTokens = [];
    private bool rendering;

    /// <param name="firstRun">The first run's polish, whose consent Use asks first; null in Settings.</param>
    /// <param name="local">This PC's own model, offered in the picker (Settings); null: own-key providers only.</param>
    public LanguageModelRows(CloudModel cloud, PolishModel? firstRun = null, LocalLlmModel? local = null)
    {
        ArgumentNullException.ThrowIfNull(cloud);
        this.cloud = cloud;
        this.firstRun = firstRun;
        this.local = firstRun is null ? local : null;
        InitializeComponent();
        ModelBox.RegisterPropertyChangedCallback(ComboBox.TextProperty, (_, _) => OnModelTyped());
        Loaded += (_, _) =>
        {
            cloud.PropertyChanged -= OnChanged;
            cloud.PropertyChanged += OnChanged;
            if (this.local is not null)
            {
                // Its download's progress and the catalogue's answers.
                this.local.PropertyChanged -= OnChanged;
                this.local.PropertyChanged += OnChanged;
            }
            Render();
        };
        Unloaded += (_, _) =>
        {
            cloud.PropertyChanged -= OnChanged;
            if (this.local is not null)
            {
                this.local.PropertyChanged -= OnChanged;
            }
        };
        Render();
    }

    /// <summary>Under Use: where the first-run sheet shows polish's consent step when this Use asked.</summary>
    public Panel StepSlot => FirstRunStepSlot;

    /// <summary>Use, where the first-run sheet puts focus back once its step is answered.</summary>
    public Button Use => UseButton;

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void Render()
    {
        rendering = true;
        try
        {
            var items = new List<(string? Id, string Name)> { (null, "None: nothing leaves this PC") };
            if (local is { Offered: true })
            {
                items.Add((CloudModel.OnDeviceId, $"On this PC: {local.Name}"));
            }
            items.AddRange(cloud.OwnKeyProviders.Select(p => ((string?)p.Id, p.Name)));
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
            var onDevice = provider?.IsOnDevice == true;
            RenderLocal(onDevice);
            ProviderDetails.Visibility = Visible(provider is not null && !onDevice);
            ServerRow.Visibility = Visible(provider?.CustomUrl == true);
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

            var useNote = firstRun is null ? cloud.UseNote : cloud.FirstRunUseNote;
            UseNote.Text = useNote;
            UseButton.Content = cloud.UseLabel;
            UseButton.IsEnabled = firstRun is null ? cloud.CanUse : PolishModel.CanUseOwnKey(cloud);
            AutomationProperties.SetHelpText(UseButton, useNote);
            TestButton.IsEnabled = cloud.CanTest;
            TestButton.Content = cloud.TestLabel;
            AutomationProperties.SetName(TestButton, cloud.TestName);
            Line(TestStatus, cloud.TestMessage, cloud.TestState == CloudTestState.Failed);
            var status = cloud.Failure ?? cloud.Status;
            Line(CloudStatus, status, cloud.Failure is not null || cloud.ReadError is not null);
            AutomationProperties.SetHelpText(ProviderBox, status);
        }
        finally
        {
            rendering = false;
        }
    }

    /// <summary>This PC's own model's rows, while it is in the picker: its line, its download and the free space.</summary>
    private void RenderLocal(bool shown)
    {
        LocalPanel.Visibility = Visible(shown && local is not null);
        if (!shown || local is null || local.Row is not ModelRow row)
        {
            return;
        }
        var format = System.Globalization.CultureInfo.CurrentCulture;
        LocalLine.Text = local.Line(format) ?? "";
        LocalProgress.Visibility = Visible(local.Progress is not null);
        LocalProgress.Value = local.Progress ?? 0;
        AutomationProperties.SetName(LocalProgress, row.ProgressName);
        Line(LocalStatus, local.Status(format), local.IsProblem);
        Line(LocalFreeSpace, local.FreeSpaceText(format), false);
        LocalDownload.Visibility = Visible(local.CanDownload);
        AutomationProperties.SetName(LocalDownload, row.DownloadName(format));
        LocalRetry.Visibility = Visible(local.CanRetry);
        AutomationProperties.SetName(LocalRetry, row.RetryName);
        LocalCancel.Visibility = Visible(local.CanCancel);
        AutomationProperties.SetName(LocalCancel, row.CancelName);
        LocalRemove.Visibility = Visible(local.CanRemove);
        AutomationProperties.SetName(LocalRemove, row.RemoveName);
    }

    private void OnLocalDownload(object sender, RoutedEventArgs e) => local?.Download();

    private void OnLocalCancel(object sender, RoutedEventArgs e) => local?.Cancel();

    /// <summary>Remove asks first (the same question as Settings > Models'), Cancel focused.</summary>
    private void OnLocalRemove(object sender, RoutedEventArgs e)
    {
        if (local is not null && local.Row is ModelRow row)
        {
            RemoveQuestion.Show(LocalRemove, row, local.Remove);
        }
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

    /// <summary>
    /// Delete asks first, saying which key goes and that every Inkwell on the account loses it (the
    /// Mac's words): Delete Key or Cancel, Cancel focused.
    /// </summary>
    private void OnDeleteKey(object sender, RoutedEventArgs e)
    {
        var question = new TextBlock { Text = cloud.DeleteKeyQuestion, TextWrapping = TextWrapping.Wrap, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Style = (Style)Application.Current.Resources["InkBodyStyle"] };
        var detail = new TextBlock { Text = CloudModel.DeleteKeyDetail, TextWrapping = TextWrapping.Wrap, Style = (Style)Application.Current.Resources["InkCaptionStyle"] };
        var delete = new Button { Content = CloudModel.DeleteKeyConfirm, Style = (Style)Application.Current.Resources["InkAccentButtonStyle"] };
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

    private void OnUse(object sender, RoutedEventArgs e)
    {
        if (firstRun is not null)
        {
            firstRun.UseOwnKey(cloud);
        }
        else
        {
            cloud.Use();
        }
    }

    private void OnTest(object sender, RoutedEventArgs e) => cloud.Test();
}
