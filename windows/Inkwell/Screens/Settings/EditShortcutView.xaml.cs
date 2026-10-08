// The voice-edit key is beside the feature it enables. Choosing it still asks through
// AiSettings, and recording uses the same suspended-hook and key-release path as dictation.
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class EditShortcutView : UserControl
{
    private readonly AiSettings ai;
    private readonly DictationModel dictation;
    private readonly ShortcutRecorderModel recorder;
    private readonly MeetingShortcutModel? meeting;
    private readonly ShortcutCaptureHost captureHost;
    private readonly List<string?> editTokens = [];
    private bool rendering;

    public EditShortcutView(AiSettings ai, ShortcutRecorderModel recorder, MeetingShortcutModel? meeting = null)
    {
        ArgumentNullException.ThrowIfNull(ai);
        ArgumentNullException.ThrowIfNull(recorder);
        this.ai = ai;
        dictation = ai.Dictation;
        this.recorder = recorder;
        this.meeting = meeting;
        captureHost = new ShortcutCaptureHost(recorder);
        InitializeComponent();
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
            if (recorder.Recording == ShortcutTarget.Edit || recorder.Waiting == ShortcutTarget.Edit || recorder.Checking?.Target == ShortcutTarget.Edit) recorder.Cancel();
            Capture(false);
        };
        Render();
    }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();
    private void Capture(bool on) => captureHost.Capture(XamlRoot?.Content as UIElement, on);
    private void OnRecordEdit(object sender, RoutedEventArgs e)
    {
        recorder.Announce = Announce;
        recorder.Toggle(ShortcutTarget.Edit);
    }
    private void Announce(string text)
    {
        var peer = FrameworkElementAutomationPeer.FromElement(RecordEditButton) ?? FrameworkElementAutomationPeer.CreatePeerForElement(RecordEditButton);
        peer?.RaiseNotificationEvent(AutomationNotificationKind.ActionCompleted, AutomationNotificationProcessing.ImportantMostRecent, text, "InkwellShortcutRecorder");
    }
    private void Render()
    {
        rendering = true;
        try
        {
            var edit = dictation.EditKey;
            var edits = new List<(string? Token, string Name)> { (null, "Off") };
            edits.AddRange(dictation.EditKeys.Select(k => ((string?)k.Token, k.Name)));
            if (edit is not null && !edits.Exists(k => k.Token == edit))
            {
                edits.Add((edit, recorder.Describe(edit).Name));
            }
            Fill(EditKeyBox, editTokens, edits);
            EditKeyBox.SelectedIndex = editTokens.IndexOf(edit);
            var showEditCap = edit is not null && dictation.EditKeyProblem is null;
            EditKeyCapHost.Visibility = Visible(showEditCap);
            EditKeyCap.Text = edit is null ? "" : recorder.Describe(edit).Cap;

            RecordEditButton.Content = recorder.ButtonTitle(ShortcutTarget.Edit);
            AutomationProperties.SetName(RecordEditButton, recorder.ButtonName(ShortcutTarget.Edit));
            AutomationProperties.SetHelpText(RecordEditButton, recorder.ButtonHint(ShortcutTarget.Edit));
            EditKeyBox.IsEnabled = !recorder.Busy;
            Message(EditMessage, recorder.Message(ShortcutTarget.Edit));
            Line(KeysPausedText, dictation.IsOn ? null : "Voice edit shortcuts are paused while Dictation is off. Turn Dictation on to use this shortcut.");
            Capture(recorder.Capturing is ShortcutTarget.Edit);
            Line(EditKeyProblemText, dictation.EditKeyProblemLine);
            Line(EditConsentProblemText, ai.EditConsentProblem);
        }
        finally
        {
            rendering = false;
        }
    }

    private void OnEditKeyChosen(object sender, SelectionChangedEventArgs e)
    {
        var i = EditKeyBox.SelectedIndex;
        if (!rendering && i >= 0 && i < editTokens.Count && editTokens[i] != dictation.EditKey)
        {
            if (editTokens[i] is string token && meeting?.Key == token)
            {
                Render();
                Line(EditMessage, "That key is used for meetings. Pick another, or change the meeting key first.");
                return;
            }
            ai.ChooseEditKey(editTokens[i]);
            // The pick shows only once the core holds it (or the consent step is answered).
            Render();
        }
    }

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
