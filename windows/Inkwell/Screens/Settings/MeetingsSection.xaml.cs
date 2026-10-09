// Settings > Meetings. The call policies are their model's (the default, each app's choice, and a
// stored list that can't be read, started over only after the user agrees; Always as the default,
// set only after the user agrees, each time it is chosen); the microphone is
// Settings > Sound's (SoundModel). A failed read or save is said where it was asked. The models
// read at core.ready (the aggregator's), so nothing loads here.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;

namespace Inkwell.Screens;

public sealed partial class MeetingsSection : UserControl
{
    private readonly CallPolicyModel calls;
    private readonly MeetingShortcutModel shortcut;
    private readonly DictationModel dictation;
    private readonly ShortcutRecorderModel recorder;
    private readonly ShortcutCaptureHost captureHost;
    private readonly List<string> keyTokens = [];
    private IReadOnlyList<CallApp>? shownApps;
    private CallPolicy? shownDefault;
    private bool rendering;
    private bool askingStartOver;
    private bool askingAlways;
    private bool askingRemove;

    public MeetingsSection(MeetingModel meetings, CallPolicyModel calls, MeetingShortcutModel shortcut, DictationModel dictation, ShortcutRecorderModel recorder)
    {
        Model = meetings ?? throw new ArgumentNullException(nameof(meetings));
        this.calls = calls ?? throw new ArgumentNullException(nameof(calls));
        this.shortcut = shortcut;
        this.dictation = dictation;
        this.recorder = recorder;
        captureHost = new ShortcutCaptureHost(recorder);
        InitializeComponent();
        Loaded += (_, _) => { shortcut.PropertyChanged += OnShortcutChanged; recorder.PropertyChanged += OnShortcutChanged; Render(); };
        Unloaded += (_, _) => { shortcut.PropertyChanged -= OnShortcutChanged; recorder.PropertyChanged -= OnShortcutChanged; if (recorder.Recording == ShortcutTarget.Meeting || recorder.Waiting == ShortcutTarget.Meeting || recorder.Checking?.Target == ShortcutTarget.Meeting) recorder.Cancel(); captureHost.Capture(null, false); };
        calls.PropertyChanged += (_, _) => Render();
        Render();
    }

    public MeetingModel Model { get; }

    public static string DefaultTitle => CallPolicyModel.DefaultTitle;

    public static string DefaultCaption => CallPolicyModel.DefaultCaption;

    public static string AlwaysWarning => CallPolicyModel.AlwaysWarning;

    public static string NeverHint => CallPolicyModel.NeverHint;

    public static string AppsTitle => CallPolicyModel.AppsTitle;

    public static string NoAppsText => CallPolicyModel.NoApps;

    public static string SettingsFailedText => MeetingModel.SettingsFailedText;

    private void Render()
    {
        rendering = true;
        try
        {
            RenderShortcut();
            Show(CallsFailure, calls.Failure);
            Show(Note, calls.Note);
            Show(Unreadable, calls.Unreadable is string why ? CallPolicyModel.UnreadableLine(why) : null);
            DefaultChoice.IsEnabled = calls.Default is not null;
            if (calls.Default != shownDefault)
            {
                shownDefault = calls.Default;
                DefaultChoice.SelectedIndex = calls.Default is CallPolicy policy ? CallPolicies.All.ToList().IndexOf(policy) : -1;
            }
            AlwaysWarningRow.Visibility = calls.Default == CallPolicy.Always ? Visibility.Visible : Visibility.Collapsed;
            NeverHintLine.Visibility = calls.Default == CallPolicy.Never ? Visibility.Visible : Visibility.Collapsed;
            var rows = calls.Rows;
            NoApps.Visibility = calls.Loaded && rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            AppsHost.Visibility = rows.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
            // Laid out again for a new list, or a choice shown as made (a pending one, or its answer).
            var items = rows.Select(row => new CallAppRowItem(row, calls)).ToList();
            if (!ReferenceEquals(shownApps, calls.Apps) || !items.SequenceEqual(AppRows.ItemsSource as IEnumerable<CallAppRowItem> ?? []))
            {
                shownApps = calls.Apps;
                var focus = RowFocus.Capture(AppRows, XamlRoot);
                AppRows.ItemsSource = items;
                RowFocus.Restore(AppRows, focus, item => ((CallAppRowItem)item).Id);
            }
        }
        finally
        {
            rendering = false;
        }
        if (calls.StartingOver is { } choice && !askingStartOver)
        {
            AskStartOver(choice);
        }
    }

    private void OnShortcutChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void RenderShortcut()
    {
        var tokens = new List<string> { "off" };
        tokens.AddRange(DictationModel.Keys.Select(k => k.Token));
        if (!tokens.Contains(shortcut.Key)) tokens.Add(shortcut.Key);
        if (!tokens.SequenceEqual(keyTokens))
        {
            keyTokens.Clear();
            keyTokens.AddRange(tokens);
            MeetingKeyBox.Items.Clear();
            foreach (var token in keyTokens) MeetingKeyBox.Items.Add(token == "off" ? "Off" : recorder.Describe(token).Name);
        }
        MeetingKeyBox.SelectedIndex = keyTokens.IndexOf(shortcut.Key);
        MeetingKeyBox.IsEnabled = shortcut.Loaded && !recorder.Busy;
        RecordMeetingKey.Content = recorder.ButtonTitle(ShortcutTarget.Meeting);
        AutomationProperties.SetName(RecordMeetingKey, recorder.ButtonName(ShortcutTarget.Meeting));
        AutomationProperties.SetHelpText(RecordMeetingKey, recorder.ButtonHint(ShortcutTarget.Meeting));
        Show(MeetingKeyProblem, shortcut.Problem);
        Show(MeetingKeyMessage, recorder.Message(ShortcutTarget.Meeting)?.Text);
        captureHost.Capture(XamlRoot?.Content as UIElement, recorder.Capturing == ShortcutTarget.Meeting);
    }

    private void OnMeetingKeyChosen(object sender, SelectionChangedEventArgs e)
    {
        var index = MeetingKeyBox.SelectedIndex;
        if (rendering || index < 0 || index >= keyTokens.Count || keyTokens[index] == shortcut.Key) return;
        var key = keyTokens[index];
        if (key != "off" && (key == dictation.CurrentKey || key == dictation.EditKey))
        {
            Show(MeetingKeyMessage, "That key is used for dictation or editing. Pick another.");
            rendering = true;
            MeetingKeyBox.SelectedIndex = keyTokens.IndexOf(shortcut.Key);
            rendering = false;
            return;
        }
        shortcut.SetKey(key);
    }

    private void OnRecordMeetingKey(object sender, RoutedEventArgs e)
    {
        recorder.Announce = text =>
        {
            var peer = FrameworkElementAutomationPeer.FromElement(RecordMeetingKey) ?? FrameworkElementAutomationPeer.CreatePeerForElement(RecordMeetingKey);
            peer?.RaiseNotificationEvent(AutomationNotificationKind.ActionCompleted, AutomationNotificationProcessing.ImportantMostRecent, text, "InkwellMeetingShortcutRecorder");
        };
        recorder.Toggle(ShortcutTarget.Meeting);
    }

    private static void Show(TextBlock block, string? text)
    {
        block.Text = text ?? "";
        block.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
    }

    private void OnDefaultChosen(object sender, SelectionChangedEventArgs e)
    {
        if (!rendering && DefaultChoice.SelectedIndex is >= 0 and < 3)
        {
            var policy = CallPolicies.All[DefaultChoice.SelectedIndex];
            if (calls.AsksBeforeDefault(policy))
            {
                AskAlways();
            }
            else if (policy != calls.Default)
            {
                shownDefault = policy;
                calls.SetDefault(policy);
            }
        }
    }

    // Also raised when a row is laid out with its choice: only the user's pick is a change.
    private void OnAppChosen(object sender, SelectionChangedEventArgs e)
    {
        if (rendering || sender is not ComboBox box || RowTag.Of(sender) is not string id || box.SelectedIndex is < 0 or >= 4)
        {
            return;
        }
        var choice = CallPolicies.Choices[box.SelectedIndex];
        if (calls.Rows.FirstOrDefault(r => r.Id == id) is { } row && row.Choice != choice)
        {
            calls.Choose(choice, id, CallPolicyOrigin.Settings);
        }
    }

    private async void OnAppRemove(object sender, RoutedEventArgs e)
    {
        if (askingRemove || XamlRoot is null || RowTag.Of(sender) is not string id
            || calls.Rows.FirstOrDefault(r => r.Id == id) is not { } row)
        {
            return;
        }
        askingRemove = true;
        try
        {
            // A changed default changes what removing this rule permits: ask with current words.
            while (true)
            {
                var shownDefault = calls.Default;
                var dialog = new ContentDialog
                {
                    XamlRoot = XamlRoot,
                    Title = $"Remove {row.Label.Name}?",
                    Content = new TextBlock { Text = calls.RemoveDetail(row.Label.Name), TextWrapping = TextWrapping.Wrap },
                    PrimaryButtonText = "Remove",
                    CloseButtonText = "Cancel",
                    DefaultButton = ContentDialogButton.Close,
                };
                if (await dialog.ShowAsync() != ContentDialogResult.Primary)
                {
                    return;
                }
                if (calls.RemovalDefaultChanged(shownDefault))
                {
                    continue;
                }
                calls.Remove(id);
                return;
            }
        }
        catch (Exception ex)
        {
            ScreenLog.System.Write($"the app removal step could not be shown ({ex.GetType().Name}); cancelled");
            Show(CallsFailure, "Couldn't show the removal confirmation. Nothing was removed.");
        }
        finally
        {
            askingRemove = false;
        }
    }

    /// <summary>Over a list the core can't read, a choice starts it over only after the user agrees.</summary>
    private async void AskStartOver((string App, CallChoice Choice) choice)
    {
        if (XamlRoot is null)
        {
            calls.CancelStartOver();
            return;
        }
        askingStartOver = true;
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = CallPolicyModel.StartOverTitle,
            Content = new TextBlock { Text = CallPolicyModel.StartOverDetail, TextWrapping = TextWrapping.Wrap },
            PrimaryButtonText = CallPolicyModel.StartOverButton,
            CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Close,
        };
        ContentDialogResult result;
        try
        {
            result = await dialog.ShowAsync();
        }
        catch (Exception e)
        {
            // Another dialog is up: nothing was agreed to, so nothing is sent.
            ScreenLog.System.Write($"the call list's start-over step could not be shown ({e.GetType().Name}); cancelled");
            askingStartOver = false;
            calls.CancelStartOver();
            Relayout();
            return;
        }
        askingStartOver = false;
        // The choice the dialog was shown for, whatever changed meanwhile.
        if (result == ContentDialogResult.Primary)
        {
            calls.ConfirmStartOver(choice);
        }
        else
        {
            calls.CancelStartOver();
        }
        Relayout();
    }

    /// <summary>Always as the default records every call without asking: set only after the user agrees. Cancelled, the old default shows again.</summary>
    private async void AskAlways()
    {
        if (askingAlways)
        {
            return;
        }
        if (XamlRoot is null)
        {
            ShowStoredDefault();
            return;
        }
        askingAlways = true;
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = CallPolicyModel.ConfirmAlwaysTitle,
            Content = new TextBlock { Text = CallPolicyModel.ConfirmAlwaysDetail, TextWrapping = TextWrapping.Wrap },
            PrimaryButtonText = CallPolicyModel.ConfirmAlwaysButton,
            CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Close,
        };
        ContentDialogResult result;
        try
        {
            result = await dialog.ShowAsync();
        }
        catch (Exception e)
        {
            // Another dialog is up: nothing was agreed to, so nothing is sent.
            ScreenLog.System.Write($"the default's Always step could not be shown ({e.GetType().Name}); cancelled");
            askingAlways = false;
            ShowStoredDefault();
            return;
        }
        askingAlways = false;
        if (result == ContentDialogResult.Primary)
        {
            calls.SetDefault(CallPolicy.Always);
        }
        // The radio shows what is stored now: Always when agreed, else the default it kept.
        ShowStoredDefault();
    }

    /// <summary>The default's choice shown again as stored (an Always not agreed to goes back).</summary>
    private void ShowStoredDefault()
    {
        rendering = true;
        try
        {
            shownDefault = calls.Default;
            DefaultChoice.SelectedIndex = calls.Default is CallPolicy policy ? CallPolicies.All.ToList().IndexOf(policy) : -1;
        }
        finally
        {
            rendering = false;
        }
    }

    /// <summary>The rows laid out again from the model: a pick not saved (the start over cancelled) goes back to what is stored.</summary>
    private void Relayout()
    {
        shownApps = null;
        AppRows.ItemsSource = null;
        Render();
    }
}

/// <summary>An app's row in Settings > Meetings as the template shows it: its icon and name | its choice | when it last called.</summary>
public sealed record CallAppRowItem
{
    public CallAppRowItem(CallAppRow row, CallPolicyModel calls)
    {
        ArgumentNullException.ThrowIfNull(row);
        ArgumentNullException.ThrowIfNull(calls);
        Id = row.Id;
        Label = row.Label;
        Titles = CallPolicies.Choices.Select(calls.Title).ToList();
        Selected = CallPolicies.Choices.ToList().IndexOf(row.Choice);
        Seen = CallPolicyModel.SeenCaption(row.Seen, DateTimeOffset.Now, System.Globalization.CultureInfo.CurrentCulture);
    }

    public string Id { get; }
    public AppLabel Label { get; }
    public string Name => Label.Name;
    public IReadOnlyList<string> Titles { get; }
    public int Selected { get; }
    public string? Seen { get; }

    /// <summary>The picker read aloud: "Calls in Zoom".</summary>
    public string PickerName => $"Calls in {Label.Name}";

    public string RemoveName => $"Remove {Label.Name} from the call apps list";

    public string SeenName => Seen is null ? "" : $"{Label.Name}: {Seen}";

    public bool Equals(CallAppRowItem? other) =>
        other is not null && Id == other.Id && Label == other.Label && Selected == other.Selected && Seen == other.Seen
        && Titles.SequenceEqual(other.Titles);

    public override int GetHashCode() => HashCode.Combine(Id, Selected);
}
