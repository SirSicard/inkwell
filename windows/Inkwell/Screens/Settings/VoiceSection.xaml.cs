// Settings > Voice. The pickers show what the core holds (a pick is sent, and the core's answer is
// what shows), so a pick reads back to the key that works until the core has rebound. Choosing an
// edit key goes through AiSettings.ChooseEditKey: from Off it asks first, and the consent step is
// shown by AiSection's ConsentDialog (both sections are on the Settings screen).
//
// "Record a shortcut…" beside each picker (ShortcutRecorderModel): while it records, this window's
// key events go to it (PreviewKeyDown/Up on the window's content, taken only while recording),
// named by side as the core names keys; the app cancels it when the window loses focus or hides,
// and leaving the section cancels it too. What it says is announced to Narrator once.
using Inkwell.Core;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using VirtualKey = Windows.System.VirtualKey;

namespace Inkwell.Screens;

public sealed partial class VoiceSection : UserControl
{
    private readonly DictationModel dictation;
    private readonly AiSettings ai;
    private readonly ShortcutRecorderModel recorder;
    /// <summary>The window's content while a shortcut is recorded (its key events are the recorder's).</summary>
    private UIElement? capturing;
    /// <summary>The tokens behind the pickers' items, in order (the edit picker's first item is Off).</summary>
    private readonly List<string?> keyTokens = [];
    private readonly List<string?> editTokens = [];
    private bool rendering;

    /// <param name="importNote">The Inkwell 0.2 key note's view and the import's row, shown under the keys, if any.</param>
    public VoiceSection(AiSettings ai, ShortcutRecorderModel recorder, UIElement? importNote = null)
    {
        ArgumentNullException.ThrowIfNull(ai);
        ArgumentNullException.ThrowIfNull(recorder);
        this.ai = ai;
        this.recorder = recorder;
        dictation = ai.Dictation;
        InitializeComponent();
        ImportNoteHost.Content = importNote;
        recorder.Describe = token => KeyNotation.Describe(token, KeyboardLayout.Character);
        recorder.Announce = Announce;
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
            recorder.Cancel();
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

    /// <summary>Takes (or gives back) the window's key events for the recorder.</summary>
    private void Capture(bool on)
    {
        if (on && capturing is null && XamlRoot?.Content is UIElement content)
        {
            capturing = content;
            content.PreviewKeyDown += OnPreviewKeyDown;
            content.PreviewKeyUp += OnPreviewKeyUp;
        }
        else if (!on && capturing is not null)
        {
            capturing.PreviewKeyDown -= OnPreviewKeyDown;
            capturing.PreviewKeyUp -= OnPreviewKeyUp;
            capturing = null;
        }
    }

    private void OnPreviewKeyDown(object sender, KeyRoutedEventArgs e) =>
        e.Handled = recorder.Feed(new ShortcutCapture.Input.KeyDown(Key(e), e.KeyStatus.WasKeyDown));

    private void OnPreviewKeyUp(object sender, KeyRoutedEventArgs e) =>
        e.Handled = recorder.Feed(new ShortcutCapture.Input.KeyUp(Key(e)));

    /// <summary>A key by its side, as the core names modifiers: right Ctrl and right Alt are extended keys, right Shift is scan code 0x36.</summary>
    private static CapturedKey Key(KeyRoutedEventArgs e)
    {
        var extended = e.KeyStatus.IsExtendedKey;
        return e.Key switch
        {
            VirtualKey.Control or VirtualKey.LeftControl or VirtualKey.RightControl =>
                CapturedKey.Side(e.Key == VirtualKey.RightControl || (e.Key == VirtualKey.Control && extended) ? "right_control" : "left_control"),
            VirtualKey.Menu or VirtualKey.LeftMenu or VirtualKey.RightMenu =>
                CapturedKey.Side(e.Key == VirtualKey.RightMenu || (e.Key == VirtualKey.Menu && extended) ? "right_alt" : "left_alt"),
            VirtualKey.Shift or VirtualKey.LeftShift or VirtualKey.RightShift =>
                CapturedKey.Side(e.Key == VirtualKey.RightShift || (e.Key == VirtualKey.Shift && e.KeyStatus.ScanCode == 0x36) ? "right_shift" : "left_shift"),
            VirtualKey.LeftWindows => CapturedKey.Side("left_win"),
            VirtualKey.RightWindows => CapturedKey.Side("right_win"),
            var key => CapturedKey.Of((uint)key),
        };
    }

    private void OnRecordKey(object sender, RoutedEventArgs e) => recorder.Toggle(ShortcutTarget.Dictation);

    private void OnRecordEdit(object sender, RoutedEventArgs e) => recorder.Toggle(ShortcutTarget.Edit);

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
            EditKeyCapBorder.Visibility = Visible(showEditCap);
            EditKeyCap.Text = edit is null ? "" : recorder.Describe(edit).Cap;

            RecordKeyButton.Content = recorder.ButtonTitle(ShortcutTarget.Dictation);
            RecordEditButton.Content = recorder.ButtonTitle(ShortcutTarget.Edit);
            Message(KeyMessage, recorder.Message(ShortcutTarget.Dictation));
            Message(EditMessage, recorder.Message(ShortcutTarget.Edit));
            Capture(recorder.Recording is not null);

            StatusText.Text = dictation.StatusLine;
            StatusText.Style = (Style)Application.Current.Resources[dictation.IsProblem ? "InkAlertTextStyle" : "InkCaptionStyle"];
            AutomationProperties.SetHelpText(DictationSwitch, dictation.StatusLine);
            RetryButton.Content = dictation.RetryTitle;
            RetryButton.Visibility = Visible(dictation.CanRetry);
            Line(EditKeyProblemText, dictation.EditKeyProblemLine);
            Line(EditConsentProblemText, ai.EditConsentProblem);
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
            dictation.SetKey(token);
        }
    }

    private void OnEditKeyChosen(object sender, SelectionChangedEventArgs e)
    {
        var i = EditKeyBox.SelectedIndex;
        if (!rendering && i >= 0 && i < editTokens.Count && editTokens[i] != dictation.EditKey)
        {
            ai.ChooseEditKey(editTokens[i]);
            // The pick shows only once the core holds it (or the consent step is answered).
            Render();
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
