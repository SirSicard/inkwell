// The Live screen's view: renders the store's live meeting and the LiveModel on their change
// signals, and hands the notes editor's text and caret paragraph to the model (NotesEdited on
// every edit and on moving to another line, NotesLeft when the editor loses focus). The words
// shown and the rules behind them are the models' (LiveModel, LiveHeader, LiveLine); this file
// only places them.
using System.Collections.Immutable;
using System.Collections.ObjectModel;
using Inkwell.Core;
using Inkwell.Core.Screens;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using VirtualKey = Windows.System.VirtualKey;
using VirtualKeyModifiers = Windows.System.VirtualKeyModifiers;

namespace Inkwell.Screens;

public sealed partial class LiveScreen : UserControl
{
    /// <summary>VK_OEM_PERIOD: Ctrl+. stops, as ⌘. does on the Mac.</summary>
    private const VirtualKey PeriodKey = (VirtualKey)190;

    private readonly CoreStore store;
    private readonly LiveModel live;
    private readonly MeetingModel meetings;
    /// <summary>The ledger as shown, kept in step with the model's by the fewest changes.</summary>
    private readonly ObservableCollection<LiveLine> ledger = [];
    /// <summary>The header's clock: runs only while this screen is loaded and the meeting still records.</summary>
    private readonly DispatcherQueueTimer clock;
    private readonly WindowPresence presence;
    private string? shownRecord;
    private int? lastParagraph;
    private bool loaded;
    private ImmutableList<StackedQuestion>? shownQuestions;
    private ImmutableList<AskedQuestion>? shownAsked;

    /// <param name="meetings">Stop, Record now and their failures (the meetings model, Settings' area).</param>
    /// <param name="presence">Whether the window is on screen: the clock stops while it is hidden to the tray.</param>
    public LiveScreen(CoreStore store, LiveModel live, MeetingModel meetings, WindowPresence presence)
    {
        ArgumentNullException.ThrowIfNull(presence);
        this.presence = presence;
        ArgumentNullException.ThrowIfNull(store);
        ArgumentNullException.ThrowIfNull(live);
        ArgumentNullException.ThrowIfNull(meetings);
        this.store = store;
        this.live = live;
        this.meetings = meetings;
        InitializeComponent();
        NotesColumn.MinWidth = LiveLayout.NotesMinWidth + LiveLayout.ColumnGutter;
        NotesArea.Margin = new Thickness(0, 0, LiveLayout.ColumnGutter, 0);
        LedgerColumn.MinWidth = LiveLayout.LedgerMinWidth + LiveLayout.ColumnGutter;
        LedgerArea.Margin = new Thickness(LiveLayout.ColumnGutter, 0, 0, 0);
        LedgerRows.ItemsSource = ledger;
        // The notes typed before the screen was last made: taken once, never while the user types.
        NotesBox.Text = live.NotesText;
        shownRecord = live.Record;

        clock = DispatcherQueue.CreateTimer();
        clock.Interval = TimeSpan.FromSeconds(1);
        clock.IsRepeating = true;
        clock.Tick += (_, _) => ShowStatus();

        Accelerator(VirtualKey.I, () => AskBox.Focus(FocusState.Keyboard));
        for (var slot = 0; slot < FarEndQuestions.Capacity; slot++)
        {
            var s = slot;
            Accelerator(VirtualKey.Number1 + slot, () => live.AnswerStacked(s));
        }
        Accelerator(PeriodKey, () =>
        {
            if (store.Meeting is { Stopping: false })
            {
                meetings.Stop();
            }
        });

        store.PropertyChanged += (_, _) => Render();
        live.PropertyChanged += (_, _) => Render();
        meetings.PropertyChanged += (_, _) => Render();
        // Shown again: the header draws once (Render), then the clock runs; hidden, it stops.
        Loaded += (_, _) =>
        {
            loaded = true;
            presence.PropertyChanged += OnPresenceChanged;
            Render();
        };
        Unloaded += (_, _) =>
        {
            loaded = false;
            presence.PropertyChanged -= OnPresenceChanged;
            clock.Stop();
        };
        Render();
    }

    private void OnPresenceChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e) => Render();

    private void Accelerator(VirtualKey key, Action action)
    {
        var accelerator = new KeyboardAccelerator { Key = key, Modifiers = VirtualKeyModifiers.Control };
        accelerator.Invoked += (_, e) =>
        {
            e.Handled = true;
            action();
        };
        KeyboardAccelerators.Add(accelerator);
    }

    private static Style Styled(string key) => (Style)Application.Current.Resources[key];

    private static void Show(TextBlock block, string? text)
    {
        block.Text = text ?? "";
        block.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
    }

    private void Render()
    {
        var meeting = store.Meeting;
        Idle.Visibility = meeting is null ? Visibility.Visible : Visibility.Collapsed;
        MeetingView.Visibility = meeting is null ? Visibility.Collapsed : Visibility.Visible;
        Show(RecordNowFailure, meetings.FailureOn(MeetingPlace.RecordNow));
        if (meeting is null)
        {
            clock.Stop();
            return;
        }

        // A new meeting starts with empty notes (the model's); the old text never carries over.
        if (live.Record is string record && record != shownRecord)
        {
            shownRecord = record;
            lastParagraph = null;
            NotesBox.Text = live.NotesText;
        }

        MeetingTitle.Text = LiveHeader.Title(meeting);
        Show(AppLine, LiveHeader.AppLine(meeting));
        Show(StartedLine, live.StartedLine);
        Show(MicLine, LiveHeader.MicLine(meeting));
        SideWarnings.Children.Clear();
        foreach (var warning in LiveHeader.SideWarnings(meeting))
        {
            SideWarnings.Children.Add(new TextBlock { Text = warning, Style = Styled("InkAlertTextStyle") });
        }
        var far = LiveHeader.FarEnd(meeting);
        Show(FarEndLine, far is { Alert: false } ? far.Text : null);
        Show(FarEndAlert, far is { Alert: true } ? far.Text : null);
        StopButton.Visibility = meeting.Stopping ? Visibility.Collapsed : Visibility.Visible;
        Show(StopFailure, meetings.FailureOn(MeetingPlace.LiveStop));
        ShowStatus();
        var ticks = ScreenClock.Runs(loaded, presence, moving: !meeting.Stopping && live.StartedAt is not null);
        if (ticks && !clock.IsRunning)
        {
            clock.Start();
        }
        else if (!ticks)
        {
            clock.Stop();
        }

        RenderLedger(meeting);
        RenderStack();
        RenderAsked();
    }

    /// <summary>The status line: "Recording · 12:41" with the red dot, or "Blotting…".</summary>
    private void ShowStatus()
    {
        if (store.Meeting is not LiveMeeting meeting)
        {
            return;
        }
        Show(StatusLine, live.StatusText(meeting));
        RecordingDot.Visibility = !meeting.Stopping && live.StartedAt is not null ? Visibility.Visible : Visibility.Collapsed;
    }

    private void RenderLedger(LiveMeeting meeting)
    {
        var lines = LiveLine.Ledger(meeting);
        var lastBefore = ledger.Count > 0 ? ledger[^1] : null;
        ListSync.Sync(ledger, lines, SameLine);
        WaitingLine.Visibility = lines.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        EarlierLine.Visibility = LiveHeader.EarlierInRecord(meeting) ? Visibility.Visible : Visibility.Collapsed;
        if (ledger.Count > 0 && !ReferenceEquals(ledger[^1], lastBefore))
        {
            // The newest line in view, as the Mac scrolls to it.
            LedgerScroll.UpdateLayout();
            LedgerScroll.ChangeView(null, LedgerScroll.ScrollableHeight, null, true);
        }
    }

    /// <summary>The same line on screen: its id counts the finals held, which shifts as the oldest are let go of.</summary>
    private static bool SameLine(LiveLine a, LiveLine b) =>
        a.Channel == b.Channel && a.AtMs == b.AtMs && a.Wet == b.Wet && a.Text == b.Text;

    /// <summary>"Asked of you": each question a button, Ctrl+1 the newest.</summary>
    private void RenderStack()
    {
        var questions = live.Stack.Questions;
        StackArea.Visibility = questions.IsEmpty ? Visibility.Collapsed : Visibility.Visible;
        if (ReferenceEquals(questions, shownQuestions))
        {
            return;
        }
        shownQuestions = questions;
        StackButtons.Children.Clear();
        for (var slot = 0; slot < questions.Count; slot++)
        {
            var s = slot;
            var key = FarEndQuestions.KeyLabel(slot);
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            row.Children.Add(new TextBlock { Text = key, Style = Styled("InkTimestampStyle"), VerticalAlignment = VerticalAlignment.Center });
            row.Children.Add(new TextBlock
            {
                Text = questions[slot].Text,
                // The newest in the text colour, the older ones secondary.
                Style = Styled(slot == 0 ? "InkBodyStyle" : "InkCaptionStyle"),
                MaxLines = 2,
                TextTrimming = TextTrimming.CharacterEllipsis,
            });
            var button = new Button { Content = row, HorizontalContentAlignment = HorizontalAlignment.Left };
            AutomationProperties.SetName(button, $"Answer: {questions[slot].Text}");
            AutomationProperties.SetHelpText(button, key);
            button.Click += (_, _) => live.AnswerStacked(s);
            StackButtons.Children.Add(button);
        }
    }

    /// <summary>The newest two questions asked and their answers ("Thinking…" while waiting).</summary>
    private void RenderAsked()
    {
        if (ReferenceEquals(live.Asked, shownAsked))
        {
            return;
        }
        shownAsked = live.Asked;
        AskedList.Children.Clear();
        foreach (var asked in live.RecentAsked)
        {
            var item = new StackPanel { Spacing = 2 };
            item.Children.Add(new TextBlock { Text = asked.Question, Style = Styled("InkBodyStyle"), FontSize = 13, FontWeight = FontWeights.SemiBold });
            // The model's words as plain text: a TextBlock parses nothing and opens no link.
            item.Children.Add(asked.IsAnswer
                ? new TextBlock { Text = asked.AnswerText, Style = Styled("InkBodyStyle"), FontSize = 13, IsTextSelectionEnabled = true }
                : new TextBlock { Text = asked.AnswerText, Style = Styled("InkCaptionStyle") });
            AskedList.Children.Add(item);
        }
    }

    // MARK: - The notes editor

    private void ReportNotes()
    {
        var paragraph = LiveModel.ParagraphOf(NotesBox.SelectionStart, NotesBox.Text);
        lastParagraph = paragraph;
        live.NotesEdited(NotesBox.Text, paragraph);
    }

    private void OnNotesChanged(object sender, TextChangedEventArgs e) => ReportNotes();

    /// <summary>Moving to another line is when the line left is saved.</summary>
    private void OnNotesSelectionChanged(object sender, RoutedEventArgs e)
    {
        if (LiveModel.ParagraphOf(NotesBox.SelectionStart, NotesBox.Text) != lastParagraph)
        {
            ReportNotes();
        }
    }

    private void OnNotesLeft(object sender, RoutedEventArgs e)
    {
        lastParagraph = null;
        live.NotesLeft();
    }

    // MARK: - Ask, Stop, Record now

    private void OnAskChanged(object sender, TextChangedEventArgs e) => live.AskText = AskBox.Text;

    private void OnAskKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key != VirtualKey.Enter)
        {
            return;
        }
        e.Handled = true;
        live.SubmitAsk();
        AskBox.Text = live.AskText;
    }

    private void OnStop(object sender, RoutedEventArgs e) => meetings.Stop();

    private void OnRecordNow(object sender, RoutedEventArgs e) => meetings.RecordNow();
}

/// <summary>The Live screen's x:Bind functions.</summary>
public static class LiveViews
{
    public static Visibility Visible(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;

    public static Visibility Collapsed(bool hidden) => hidden ? Visibility.Collapsed : Visibility.Visible;

    /// <summary>A wet line's dot is fainter (the Mac's 0.6).</summary>
    public static double DotOpacity(bool wet) => wet ? 0.6 : 1;

    /// <summary>A wet line's speaker is fainter (the Mac's 0.8).</summary>
    public static double SpeakerOpacity(bool wet) => wet ? 0.8 : 1;
}
