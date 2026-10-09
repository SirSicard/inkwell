// The mode editor, as a ContentDialog (the Mac's sheet): the name, how it writes, its switches, its
// model, its polish instructions and the apps it is used in. Nothing is saved until Save; Cancel or
// Escape leaves the mode as it was. Save keeps the dialog open until the core answers: the model
// closes it (ModesModel.Editor goes), or a refusal is said at the top, in words chosen by its code.
//
// A mode's own model at a destination no polish consent covers asks for its OK first. WinUI shows
// one ContentDialog at a time, so that step is a card inside the editor (as the first run's), and
// Save waits while it is up. The dialog is at most 548 epx wide (ContentDialog's widest): its rows
// keep their name beside the controls there (ModesLayout), and what does not fit scrolls.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Windows.Storage.Pickers;

namespace Inkwell.Screens;

public sealed class ModeEditorDialog
{
    private readonly ModesModel modes;
    private readonly ModeEditor editor;
    private readonly Func<nint> windowHandle;
    private readonly ContentDialog dialog;
    private bool rendering;

    // The parts that follow the model.
    private readonly TextBlock error = Caption("", alert: true);
    private readonly TextBox name = new() { PlaceholderText = "Chat, Email, Notes…" };
    private readonly TextBlock renamed = Caption("Voice commands use the new name.");
    private readonly ComboBox style = new() { MinWidth = 170 };
    private readonly ToggleSwitch fillers = Switch("Clean up speech");
    private readonly ToggleSwitch polish = Switch("Polish");
    private readonly TextBlock polishNote = Caption("", alert: true);
    private readonly ComboBox model = new() { HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly TextBox modelName = new();
    private readonly TextBlock modelNameCaption = Caption("Blank uses the model chosen in AI.");
    private readonly TextBlock modelNote = Caption("");
    private readonly Button confirm = new() { Content = "Confirm" };
    private readonly TextBlock confirmed = Caption("Confirmed. Saving records where it sends now.");
    private readonly TextBox prompt = new()
    {
        AcceptsReturn = true,
        TextWrapping = TextWrapping.Wrap,
        Height = 120,
    };
    private readonly Button useDefault = new() { Content = "Use the default" };
    private readonly TextBlock count = Caption("");
    private readonly StackPanel apps = new() { Spacing = 6 };
    private readonly StackPanel moving = new() { Spacing = 2 };
    private readonly DropDownButton addApp = new() { Content = "Add an app" };
    private readonly Border consent = new() { Visibility = Visibility.Collapsed };
    private readonly TextBlock consentTitle = new() { FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap };
    private readonly TextBlock consentMessage = Caption("");
    private readonly Button consentCancel = new() { Content = "Cancel" };
    private readonly Button consentAllow = new();
    private IReadOnlyList<(string? Id, string Label)> options = [];
    private IReadOnlyList<string> shownApps = [];

    private ModeEditorDialog(ModesModel modes, ModeEditor editor, XamlRoot root, ElementTheme theme, Func<nint> windowHandle)
    {
        this.modes = modes;
        this.editor = editor;
        this.windowHandle = windowHandle;
        dialog = new ContentDialog
        {
            XamlRoot = root,
            // A dialog does not take the window's theme: the appearance shown now.
            RequestedTheme = theme,
            Title = editor.Adding ? "Add a mode" : $"Edit “{editor.Original?.Name ?? editor.Name}”",
            PrimaryButtonText = "Save",
            CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Primary,
            Content = Build(),
        };
        dialog.Resources["ContentDialogMaxWidth"] = ModesLayout.DialogMaxWidth;
        dialog.Opened += (_, _) => name.Focus(FocusState.Programmatic);
        dialog.PrimaryButtonClick += (_, args) =>
        {
            // Open until the core answers: the model closes it, or says why not.
            args.Cancel = true;
            modes.Save();
        };
        Wire();
    }

    /// <summary>Shows the editor for <paramref name="editor"/>; done when it closes.</summary>
    public static async Task Show(ModesModel modes, ModeEditor editor, XamlRoot root, ElementTheme theme, Func<nint> windowHandle)
    {
        ArgumentNullException.ThrowIfNull(modes);
        ArgumentNullException.ThrowIfNull(editor);
        var shown = new ModeEditorDialog(modes, editor, root, theme, windowHandle);
        modes.PropertyChanged += shown.OnChanged;
        editor.PropertyChanged += shown.OnChanged;
        try
        {
            shown.Render();
            await shown.dialog.ShowAsync();
        }
        finally
        {
            modes.PropertyChanged -= shown.OnChanged;
            editor.PropertyChanged -= shown.OnChanged;
            // Cancel, Escape, or closed by the model once saved: if it is still the editor, it goes.
            if (ReferenceEquals(modes.Editor, editor))
            {
                modes.CloseEditor();
            }
        }
    }

    private void OnChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        if (!ReferenceEquals(modes.Editor, editor))
        {
            // Saved (or closed elsewhere): the dialog goes.
            dialog.Hide();
            return;
        }
        Render();
    }

    private static Style App(string key) => (Style)Application.Current.Resources[key];

    private static TextBlock Caption(string text, bool alert = false) =>
        new() { Text = text, Style = App(alert ? "InkAlertTextStyle" : "InkCaptionStyle"), TextWrapping = TextWrapping.Wrap };

    private static ToggleSwitch Switch(string name)
    {
        var toggle = new ToggleSwitch { OnContent = "", OffContent = "", MinWidth = 0 };
        AutomationProperties.SetName(toggle, name);
        return toggle;
    }

    private static SettingColumnsPanel Row(string title, params UIElement[] controls)
    {
        var stack = new StackPanel { Spacing = 6 };
        foreach (var control in controls)
        {
            stack.Children.Add(control);
        }
        var columns = new SettingColumnsPanel { TitleColumn = ModesLayout.EditorTitleColumn };
        columns.Children.Add(new TextBlock { Text = title, Style = App("InkBodyStyle"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 6, 0, 0) });
        columns.Children.Add(stack);
        return columns;
    }

    private StackPanel Build()
    {
        AutomationProperties.SetLiveSetting(error, AutomationLiveSetting.Assertive);
        AutomationProperties.SetName(name, "Name");
        AutomationProperties.SetName(style, "Writes");
        AutomationProperties.SetName(model, "Polish with");
        AutomationProperties.SetName(modelName, "Model at the provider");
        AutomationProperties.SetName(prompt, "Polish instructions");
        AutomationProperties.SetHelpText(prompt, "Blank uses the default.");
        AutomationProperties.SetName(confirm, "Confirm where this mode's model sends");
        AutomationProperties.SetName(addApp, "Add an app");
        prompt.PlaceholderText = modes.DefaultPrompt;

        var countRow = new Grid { ColumnSpacing = 8 };
        countRow.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        countRow.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        countRow.Children.Add(useDefault);
        count.HorizontalAlignment = HorizontalAlignment.Right;
        count.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(count, 1);
        countRow.Children.Add(count);

        var flyout = new MenuFlyout();
        flyout.Opening += (_, _) => FillAddMenu(flyout);
        addApp.Flyout = flyout;

        consent.Style = App("InkInsetCardStyle");
        consent.Padding = new Thickness(14);
        var consentButtons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        consentAllow.Style = App("InkAccentButtonStyle");
        consentButtons.Children.Add(consentCancel);
        consentButtons.Children.Add(consentAllow);
        consent.Child = new StackPanel { Spacing = 8, Children = { consentTitle, consentMessage, consentButtons } };
        AutomationProperties.SetLiveSetting(consent, AutomationLiveSetting.Assertive);

        var content = new StackPanel
        {
            Spacing = 14,
            Width = ModesLayout.DialogMaxWidth - 2 * ModesLayout.DialogPadding,
            Padding = new Thickness(0, 0, ModesLayout.EditorScrollGutter, 0),
        };
        content.Children.Add(error);
        content.Children.Add(Row("Name", name, Caption("Say its name in a voice command to switch to it."), renamed));
        content.Children.Add(Row("Writes", style));
        content.Children.Add(Row("Clean up speech", fillers, Caption("Takes out fillers and stutters.")));
        content.Children.Add(Row("Polish", polish, Caption("Tidies the wording with a language model before it is typed."), polishNote));
        content.Children.Add(Row("Polish with", model, modelName, modelNameCaption, modelNote, confirm, confirmed));
        content.Children.Add(Row("Polish instructions", prompt, countRow, Caption("Blank uses the default.")));
        content.Children.Add(Row("Used in", apps, moving, addApp));
        content.Children.Add(consent);
        return content;
    }

    private void Wire()
    {
        name.TextChanged += (_, _) => { if (!rendering) { editor.Name = name.Text; } };
        style.SelectionChanged += (_, _) =>
        {
            if (!rendering && style.SelectedItem is ComboBoxItem { Tag: ModeStyle picked })
            {
                editor.Style = picked;
            }
        };
        fillers.Toggled += (_, _) => { if (!rendering) { editor.RemoveFillers = fillers.IsOn; } };
        polish.Toggled += (_, _) => { if (!rendering) { editor.Polish = polish.IsOn; } };
        model.SelectionChanged += (_, _) =>
        {
            if (!rendering && model.SelectedIndex >= 0 && model.SelectedIndex < options.Count)
            {
                editor.PolishModel = options[model.SelectedIndex].Id;
            }
        };
        modelName.TextChanged += (_, _) => { if (!rendering) { editor.PolishModelName = modelName.Text; } };
        // As typed, line breaks and all: the core judges whether it is the default.
        prompt.TextChanged += (_, _) => { if (!rendering) { editor.Prompt = prompt.Text; } };
        useDefault.Click += (_, _) => editor.Prompt = "";
        confirm.Click += (_, _) => modes.ConfirmInEditor(editor);
        consentCancel.Click += (_, _) => modes.CancelConsentStep();
        consentAllow.Click += (_, _) =>
        {
            if (editor.ConsentStep is ConsentDestination destination)
            {
                modes.AllowAndSave(destination);
            }
        };
    }

    private void Render()
    {
        rendering = true;
        try
        {
            error.Text = editor.Error ?? "";
            error.Visibility = editor.Error is null ? Visibility.Collapsed : Visibility.Visible;
            if (name.Text != editor.Name)
            {
                name.Text = editor.Name;
            }
            renamed.Visibility = editor.Renamed ? Visibility.Visible : Visibility.Collapsed;
            RenderStyle();
            fillers.IsOn = editor.RemoveFillers;
            polish.IsOn = editor.Polish;
            polishNote.Text = modes.PolishNote(editor) ?? "";
            polishNote.Visibility = polishNote.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
            RenderModel();
            if (prompt.Text != editor.Prompt)
            {
                prompt.Text = editor.Prompt;
            }
            useDefault.IsEnabled = editor.Prompt.Length > 0;
            count.Text = editor.PromptCount;
            count.Style = App(editor.PromptTooLong ? "InkAlertTextStyle" : "InkCaptionStyle");
            AutomationProperties.SetName(count, $"{ModesModel.Count(editor.Prompt)} of {ModesModel.PromptLimit} characters");
            RenderApps();
            RenderConsent();
            dialog.IsPrimaryButtonEnabled = !editor.Saving && editor.ConsentStep is null;
        }
        finally
        {
            rendering = false;
        }
    }

    private void RenderStyle()
    {
        if (style.Items.Count == 0)
        {
            var styles = new List<ModeStyle> { ModeStyle.Formal, ModeStyle.Casual, ModeStyle.Relaxed };
            // A style this build does not know stays until another is picked.
            if (editor.Original?.Style == ModeStyle.Other)
            {
                styles.Add(ModeStyle.Other);
            }
            foreach (var s in styles)
            {
                style.Items.Add(new ComboBoxItem { Content = ModesModel.Style(s), Tag = s });
            }
        }
        style.SelectedItem = style.Items.OfType<ComboBoxItem>().FirstOrDefault(i => i.Tag is ModeStyle s && s == editor.Style);
    }

    private void RenderModel()
    {
        var now = modes.ModelOptions(editor);
        if (!now.SequenceEqual(options))
        {
            options = now;
            model.Items.Clear();
            foreach (var option in options)
            {
                model.Items.Add(option.Label);
            }
        }
        model.SelectedIndex = options.ToList().FindIndex(o => o.Id == editor.PolishModel);
        var provider = editor.PolishModel?.StartsWith("provider:", StringComparison.Ordinal) == true;
        modelName.Visibility = modelNameCaption.Visibility = provider ? Visibility.Visible : Visibility.Collapsed;
        modelName.PlaceholderText = modes.ProviderModel(editor) ?? "The model chosen in AI";
        if (modelName.Text != editor.PolishModelName)
        {
            modelName.Text = editor.PolishModelName;
        }
        var (text, problem) = modes.ModelNote(editor);
        modelNote.Text = text;
        modelNote.Style = App(problem ? "InkAlertTextStyle" : "InkCaptionStyle");
        confirm.Visibility = modes.CanConfirmInEditor(editor) ? Visibility.Visible : Visibility.Collapsed;
        confirmed.Visibility = modes.Confirmed(editor) ? Visibility.Visible : Visibility.Collapsed;
    }

    private void RenderApps()
    {
        if (editor.IsDefault)
        {
            addApp.Visibility = Visibility.Collapsed;
            if (apps.Children.Count == 0)
            {
                apps.Children.Add(Caption("Every app without a mode of its own"));
            }
            return;
        }
        if (!shownApps.SequenceEqual(editor.Apps))
        {
            shownApps = [.. editor.Apps];
            apps.Children.Clear();
            if (editor.Apps.Count == 0)
            {
                apps.Children.Add(Caption("No apps: used only when you switch to it by voice."));
            }
            foreach (var identity in editor.Apps)
            {
                apps.Children.Add(AppRow(identity));
            }
        }
        moving.Children.Clear();
        foreach (var note in modes.MovingNotes(editor))
        {
            moving.Children.Add(Caption(note));
        }
    }

    /// <summary>An app in the list: its icon, its name, and Remove.</summary>
    private Grid AppRow(string identity)
    {
        var label = modes.Label(identity);
        var row = new Grid { ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var tile = new AppIconTile { App = label };
        AutomationProperties.SetAccessibilityView(tile, AccessibilityView.Raw);
        var text = new TextBlock { Text = label.Name, Style = App("InkBodyStyle"), VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis };
        Grid.SetColumn(text, 1);
        var remove = new Button { Content = new SymbolIcon(Symbol.Remove), Padding = new Thickness(6) };
        AutomationProperties.SetName(remove, $"Remove {label.Name}");
        ToolTipService.SetToolTip(remove, $"Remove {label.Name}");
        remove.Click += (_, _) => ModesModel.RemoveApp(identity, editor);
        Grid.SetColumn(remove, 2);
        row.Children.Add(tile);
        row.Children.Add(text);
        row.Children.Add(remove);
        return row;
    }

    /// <summary>"Running now" (each in another mode says which; picking it moves it), then Browse….</summary>
    private void FillAddMenu(MenuFlyout flyout)
    {
        flyout.Items.Clear();
        flyout.Items.Add(new MenuFlyoutItem { Text = "Running now", IsEnabled = false });
        var offers = modes.RunningOffers(editor);
        if (offers.Count == 0)
        {
            flyout.Items.Add(new MenuFlyoutItem { Text = "No other apps are running", IsEnabled = false });
        }
        foreach (var (app, owner) in offers)
        {
            var item = new MenuFlyoutItem { Text = owner is null ? app.Name : $"{app.Name} — in {owner}" };
            item.Click += (_, _) => ModesModel.AddApp(app.Identity, editor);
            flyout.Items.Add(item);
        }
        flyout.Items.Add(new MenuFlyoutSeparator());
        var browse = new MenuFlyoutItem { Text = "Browse…" };
        browse.Click += async (_, _) => await Browse().ConfigureAwait(true);
        flyout.Items.Add(browse);
    }

    /// <summary>An app's .exe, from a file picker. (Store apps live where the picker can't go: they are in Running now.)</summary>
    private async Task Browse()
    {
        var picker = new FileOpenPicker { SuggestedStartLocation = PickerLocationId.ComputerFolder };
        picker.FileTypeFilter.Add(".exe");
        WinRT.Interop.InitializeWithWindow.Initialize(picker, windowHandle());
        try
        {
            if (await picker.PickSingleFileAsync() is { } file)
            {
                ModesModel.AddBrowsed(file.Path, editor);
            }
        }
        catch (Exception e) when (e is System.Runtime.InteropServices.COMException or UnauthorizedAccessException)
        {
            // The picker could not open: nothing was added. Logged by its kind, never a path.
            ScreenLog.System.Write($"the app picker could not open ({e.GetType().Name})");
        }
    }

    /// <summary>The OK for where the mode's model sends: inside the editor (one dialog at a time), Save waiting while it is up.</summary>
    private void RenderConsent()
    {
        if (editor.ConsentStep is not ConsentDestination destination)
        {
            consent.Visibility = Visibility.Collapsed;
            return;
        }
        var wasHidden = consent.Visibility == Visibility.Collapsed;
        consentTitle.Text = modes.ConsentTitle(editor);
        consentMessage.Text = modes.ConsentMessage(destination);
        consentAllow.Content = ConsentModel.Button(LlmFeature.Polish, destination);
        AutomationProperties.SetName(consentAllow, ConsentModel.AllowName(LlmFeature.Polish, destination));
        AutomationProperties.SetName(consentCancel, "Cancel, and save nothing");
        consent.Visibility = Visibility.Visible;
        if (wasHidden)
        {
            consent.StartBringIntoView();
            // For a model off this PC, focus (and Enter) lands on Cancel: agreeing is a deliberate press.
            (ConsentModel.FocusesCancel(destination) ? consentCancel : consentAllow).Focus(FocusState.Programmatic);
            var peer = FrameworkElementAutomationPeer.FromElement(consent) ?? FrameworkElementAutomationPeer.CreatePeerForElement(consent);
            peer?.RaiseNotificationEvent(
                AutomationNotificationKind.ActionCompleted, AutomationNotificationProcessing.ImportantMostRecent,
                $"{consentTitle.Text} {consentMessage.Text}", "mode-consent");
        }
    }
}
