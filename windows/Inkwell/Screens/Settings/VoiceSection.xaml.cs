// Settings > Voice. The pickers show what the core holds (a pick is sent, and the core's answer is
// what shows), so a pick reads back to the key that works until the core has rebound. Choosing an
// edit key goes through AiSettings.ChooseEditKey: from Off it asks first, and the consent step is
// shown by AiSection's ConsentDialog (both sections are on the Settings screen).
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Inkwell.Screens;

public sealed partial class VoiceSection : UserControl
{
    private readonly DictationModel dictation;
    private readonly AiSettings ai;
    /// <summary>The tokens behind the pickers' items, in order (the edit picker's first item is Off).</summary>
    private readonly List<string?> keyTokens = [];
    private readonly List<string?> editTokens = [];
    private bool rendering;

    /// <param name="importNote">The Inkwell 0.2 key note's view, shown under the keys, if any.</param>
    public VoiceSection(AiSettings ai, UIElement? importNote = null)
    {
        ArgumentNullException.ThrowIfNull(ai);
        this.ai = ai;
        dictation = ai.Dictation;
        InitializeComponent();
        ImportNoteHost.Content = importNote;
        Loaded += (_, _) =>
        {
            ai.PropertyChanged += OnChanged;
            Render();
        };
        Unloaded += (_, _) => ai.PropertyChanged -= OnChanged;
        Render();
    }

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
                // A chord the core holds (from Inkwell 0.2, say): shown, though not offered.
                keys.Add((key, DictationModel.Cap(key)));
            }
            Fill(KeyBox, keyTokens, keys.Select(k => ((string?)k.Token, k.Name)).ToList());
            KeyBox.SelectedIndex = keyTokens.IndexOf(key);
            KeyCap.Text = DictationModel.Cap(key);

            var edit = dictation.EditKey;
            var edits = new List<(string? Token, string Name)> { (null, "Off") };
            edits.AddRange(dictation.EditKeys.Select(k => ((string?)k.Token, k.Name)));
            if (edit is not null && !edits.Exists(k => k.Token == edit))
            {
                edits.Add((edit, DictationModel.Cap(edit)));
            }
            Fill(EditKeyBox, editTokens, edits);
            EditKeyBox.SelectedIndex = editTokens.IndexOf(edit);
            var showEditCap = edit is not null && dictation.EditKeyProblem is null;
            EditKeyCapBorder.Visibility = Visible(showEditCap);
            EditKeyCap.Text = edit is null ? "" : DictationModel.Cap(edit);

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

    private static void Line(TextBlock block, string? text)
    {
        block.Text = text ?? "";
        block.Visibility = Visible(text is not null);
    }

    private static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;
}
