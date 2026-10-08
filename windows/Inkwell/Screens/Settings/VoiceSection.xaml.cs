// Settings > Dictation. Pickers show confirmed core bindings; the shared recorder
// suspends global hooks and waits for held keys to be released before resuming them.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class VoiceSection : UserControl
{
    private readonly DictationModel dictation;
    private readonly MeetingShortcutModel? meeting;
    private readonly ShortcutRecorderModel recorder;
    /// <summary>The window's content while a shortcut is recorded (its key events are the recorder's).</summary>
    private readonly ShortcutCaptureHost captureHost;
    /// <summary>The tokens behind the dictation picker's items, in order.</summary>
    private readonly List<string?> keyTokens = [];
    private bool rendering;

    /// <param name="importNote">The Inkwell 0.2 key note's view and the import's row, shown under the keys, if any.</param>
    public VoiceSection(AiSettings ai, ShortcutRecorderModel recorder, UIElement? importNote = null, MeetingShortcutModel? meeting = null)
    {
        ArgumentNullException.ThrowIfNull(ai);
        ArgumentNullException.ThrowIfNull(recorder);
        this.meeting = meeting;
        this.recorder = recorder;
        captureHost = new ShortcutCaptureHost(recorder);
        dictation = ai.Dictation;
        InitializeComponent();
        ImportNoteHost.Content = importNote;
        Loaded += (_, _) =>
        {
            ai.PropertyChanged += OnChanged;
            recorder.PropertyChanged += OnChanged;
            Render();
        };
        Unloaded += (_, _) =>
        {
            ai.PropertyChanged -= OnChanged;
            recorder.PropertyChanged -= OnChanged;
            if (recorder.Recording == ShortcutTarget.Dictation || recorder.Waiting == ShortcutTarget.Dictation || recorder.Checking?.Target == ShortcutTarget.Dictation) recorder.Cancel();
            Capture(false);
        };
        Render();
    }

    /// <summary>
    /// Says <paramref name="text"/> to Narrator, once. Raised from the Record button's peer: it is
    /// in the automation tree (a UserControl's own peer is not, and Narrator drops what it raises).
    /// </summary>
    private void Announce(string text)
    {
        var peer = FrameworkElementAutomationPeer.FromElement(RecordKeyButton) ?? FrameworkElementAutomationPeer.CreatePeerForElement(RecordKeyButton);
        peer?.RaiseNotificationEvent(AutomationNotificationKind.ActionCompleted, AutomationNotificationProcessing.ImportantMostRecent, text, "InkwellShortcutRecorder");
    }

    private void Capture(bool on) => captureHost.Capture(XamlRoot?.Content as UIElement, on);

    private void OnRecordKey(object sender, RoutedEventArgs e) { recorder.Announce = Announce; recorder.Toggle(ShortcutTarget.Dictation); }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void Render()
    {
        rendering = true;
        try
        {
            DictationSwitch.IsOn = dictation.IsOn;
            SwitchCaption.Text = dictation.SwitchCaption;

            var key = dictation.CurrentKey;
            var keys = DictationModel.Keys.Select(k => (Token: k.Token, Name: k.Name)).ToList();
            if (!keys.Exists(k => k.Token == key))
            {
                // A recorded key (or one from Inkwell 0.2): shown in the picker, as the layout labels it.
                keys.Add((key, recorder.Describe(key).Name));
            }
            Fill(KeyBox, keyTokens, keys.Select(k => ((string?)k.Token, k.Name)).ToList());
            KeyBox.SelectedIndex = keyTokens.IndexOf(key);
            KeyCap.Text = recorder.Describe(key).Cap;

            RecordKeyButton.Content = recorder.ButtonTitle(ShortcutTarget.Dictation);
            AutomationProperties.SetName(RecordKeyButton, recorder.ButtonName(ShortcutTarget.Dictation));
            AutomationProperties.SetHelpText(RecordKeyButton, recorder.ButtonHint(ShortcutTarget.Dictation));
            // While recording, the key comes from the keyboard: the switch and the pickers wait.
            var idle = !recorder.Busy;
            DictationSwitch.IsEnabled = idle;
            KeyBox.IsEnabled = idle;
            Message(KeyMessage, recorder.Message(ShortcutTarget.Dictation));
            Capture(recorder.Capturing is ShortcutTarget.Dictation);

            StatusText.Text = dictation.StatusLine;
            StatusText.Style = (Style)Application.Current.Resources[dictation.IsProblem ? "InkAlertTextStyle" : "InkCaptionStyle"];
            AutomationProperties.SetHelpText(DictationSwitch, dictation.StatusLine);
            RetryButton.Content = dictation.RetryTitle;
            RetryButton.Visibility = Visible(dictation.CanRetry);
            Line(SettingsProblemText, dictation.SettingsProblemLine);
        }
        finally
        {
            rendering = false;
        }
    }

    private void OnDictationToggled(object sender, RoutedEventArgs e)
    {
        if (!rendering && DictationSwitch.IsOn != dictation.IsOn)
        {
            dictation.SetOn(DictationSwitch.IsOn);
        }
    }

    private void OnKeyChosen(object sender, SelectionChangedEventArgs e)
    {
        var i = KeyBox.SelectedIndex;
        if (!rendering && i >= 0 && i < keyTokens.Count && keyTokens[i] is string token && token != dictation.CurrentKey)
        {
            if (meeting?.Key == token)
            {
                Render();
                Line(KeyMessage, "That key is used for meetings. Pick another, or change the meeting key first.");
                return;
            }
            dictation.SetKey(token);
        }
    }

    private void OnRetry(object sender, RoutedEventArgs e) => dictation.Retry();

    /// <summary>Puts <paramref name="items"/> in <paramref name="box"/>, only when they changed (a rebuild would close an open list).</summary>
    private static void Fill(ComboBox box, List<string?> tokens, List<(string? Token, string Name)> items)
    {
        if (tokens.SequenceEqual(items.Select(i => i.Token)))
        {
            return;
        }
        tokens.Clear();
        box.Items.Clear();
        foreach (var (token, name) in items)
        {
            tokens.Add(token);
            box.Items.Add(name);
        }
    }

    private static void Message(TextBlock block, ShortcutMessage? message)
    {
        block.Text = message?.Text ?? "";
        block.Visibility = Visible(message is not null);
        block.Style = (Style)Application.Current.Resources[message?.IsProblem == true ? "InkAlertTextStyle" : "InkCaptionStyle"];
    }

    private static void Line(TextBlock block, string? text)
    {
        block.Text = text ?? "";
        block.Visibility = Visible(text is not null);
    }

    private static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;
}
