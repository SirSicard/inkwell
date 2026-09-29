// Today's code-behind: places what the models say, and routes the buttons. It asks for Today's
// data when the screen is loaded (the library's today, what is owed, the calendar, and the
// permission check the Settings cards share), listens to its models only while loaded, and
// redraws "in 42 min" on the minute through a one-shot DispatcherQueueTimer, only while the
// screen is loaded, the window is on screen and an event is shown (architecture rule 9).
using System.ComponentModel;
using Inkwell.Core;
using Inkwell.Core.Screens;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;

namespace Inkwell.Screens;

public sealed partial class TodayScreen : UserControl
{
    /// <summary>The content's width from which Today shows two columns (the Mac's 624 pt).</summary>
    private const double TwoColumns = 624;
    /// <summary>The horizontal padding (48 each side).</summary>
    private const double Gutters = 96;

    private readonly CoreStore store;
    private readonly LibraryModel library;
    private readonly OwedModel owed;
    private readonly PermissionsModel permissions;
    private readonly UpNextModel upNext;
    private readonly WindowPresence presence;
    private readonly Action<Route> open;
    private readonly Action<string, long?, bool> openRecord;
    private readonly DispatcherQueueTimer minute;
    private bool showAllNeeds;
    private IReadOnlyList<NeedsYouItem>? shownNeeds;
    private bool loaded;

    /// <param name="open">Opens a route (Owed from "All N").</param>
    /// <param name="openRecord">Opens a record in the Library: the record, where to put the playhead, and whether to play.</param>
    /// <param name="presence">Whether the window is on screen: Up next's minute redraws only then.</param>
    public TodayScreen(
        CoreStore store,
        LibraryModel library,
        OwedModel owed,
        PermissionsModel permissions,
        UpNextModel upNext,
        WindowPresence presence,
        Action<Route> open,
        Action<string, long?, bool> openRecord)
    {
        ArgumentNullException.ThrowIfNull(store);
        ArgumentNullException.ThrowIfNull(library);
        ArgumentNullException.ThrowIfNull(owed);
        ArgumentNullException.ThrowIfNull(permissions);
        ArgumentNullException.ThrowIfNull(upNext);
        ArgumentNullException.ThrowIfNull(presence);
        ArgumentNullException.ThrowIfNull(open);
        ArgumentNullException.ThrowIfNull(openRecord);
        this.store = store;
        this.library = library;
        this.owed = owed;
        this.permissions = permissions;
        this.upNext = upNext;
        this.presence = presence;
        this.open = open;
        this.openRecord = openRecord;
        InitializeComponent();
        minute = DispatcherQueue.GetForCurrentThread().CreateTimer();
        minute.IsRepeating = false;
        minute.Tick += (_, _) => MinuteTick();
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
        SizeChanged += (_, _) => Layout();
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        loaded = true;
        // The store changes with every batch (a meeting's partials among them): only the banner
        // reads it, so only the banner is redrawn for it.
        store.PropertyChanged += OnStoreChanged;
        foreach (var model in Models())
        {
            model.PropertyChanged += OnModelChanged;
        }
        library.RefreshToday();
        owed.Load();
        // A screen showing permissions: checked now, and again whenever the app comes back to the
        // front while it is up (the user may be back from Settings).
        permissions.ScreenAppeared();
        upNext.Refresh();
        Render();
    }

    private void OnUnloaded(object sender, RoutedEventArgs e)
    {
        loaded = false;
        store.PropertyChanged -= OnStoreChanged;
        foreach (var model in Models())
        {
            model.PropertyChanged -= OnModelChanged;
        }
        permissions.ScreenDisappeared();
        minute.Stop();
    }

    private INotifyPropertyChanged[] Models() => [library, owed, permissions, upNext, presence];

    private void OnModelChanged(object? sender, PropertyChangedEventArgs e) => Render();

    private void OnStoreChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (loaded)
        {
            RenderNeedsYou();
        }
    }

    private void Render()
    {
        if (!loaded)
        {
            return;
        }
        var now = library.Now();
        var calendar = library.Calendar;
        DateLine.Text = TodayText.LongDay(now, calendar).ToUpper(calendar.Culture);
        AutomationProperties.SetName(DateLine, TodayText.LongDay(now, calendar));
        Greeting.Text = LibraryFormat.Greeting(now, calendar);
        RenderNeedsYou();
        RenderLastMeeting(now, calendar);
        RenderUpNext(now, calendar);
        RenderOwed(now);
        RenderStats(calendar);
        Layout();
    }

    // Needs you

    private void RenderNeedsYou(bool force = false)
    {
        var items = NeedsYou.Items(permissions, library, store);
        // Unchanged (most batches): the rows stay, and so does keyboard focus on them.
        if (!force && shownNeeds is not null && shownNeeds.SequenceEqual(items))
        {
            return;
        }
        shownNeeds = items;
        NeedsYouCard.Visibility = Show(items.Count > 0);
        NeedsYouRows.Children.Clear();
        for (var i = 0; i < items.Count && (i == 0 || showAllNeeds); i++)
        {
            if (i > 0)
            {
                NeedsYouRows.Children.Add(new Rectangle { Height = 1, Fill = BrushOf("InkBorderBrush") });
            }
            NeedsYouRows.Children.Add(NeedsYouRow(items[i]));
        }
        var more = NeedsYou.MoreTitle(items.Count, showAllNeeds);
        NeedsYouMore.Content = more;
        NeedsYouMore.Visibility = Show(more is not null);
    }

    private Grid NeedsYouRow(NeedsYouItem item)
    {
        var row = new Grid { ColumnSpacing = 16 };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var icon = new FontIcon { Glyph = "", FontSize = 24, Foreground = BrushOf("InkAlertBrush"), VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetAccessibilityView(icon, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
        row.Children.Add(icon);
        var words = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
        words.Children.Add(new TextBlock { Text = item.Title, FontSize = 15, FontWeight = FontWeights.SemiBold, Style = StyleOf("InkBodyStyle") });
        words.Children.Add(new TextBlock { Text = item.Detail, Style = StyleOf("InkCaptionStyle") });
        Grid.SetColumn(words, 1);
        row.Children.Add(words);
        var button = new Button
        {
            Content = item.ActionTitle,
            Style = StyleOf("AccentButtonStyle"),
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(12, 0, 0, 0),
        };
        AutomationProperties.SetHelpText(button, item.Title);
        button.Click += (_, _) => Perform(item.Action);
        Grid.SetColumn(button, 2);
        row.Children.Add(button);
        return row;
    }

    private void Perform(NeedsYouAction action)
    {
        switch (action)
        {
            case NeedsYouAction.Allow allow:
                permissions.Request(NeedsYou.Card(allow.Permission));
                break;
            case NeedsYouAction.OpenSoundSettings:
                OpenSoundSettings();
                break;
            case NeedsYouAction.Dismiss dismiss:
                store.DismissNotice(dismiss.NoticeId);
                break;
            case NeedsYouAction.RetryChecks:
                library.RefreshToday();
                break;
            default:
                break;
        }
    }

    private void OnNeedsYouMore(object sender, RoutedEventArgs e)
    {
        showAllNeeds = !showAllNeeds;
        RenderNeedsYou(force: true);
    }

    // Last meeting

    private void RenderLastMeeting(DateTimeOffset now, LibraryCalendar calendar)
    {
        var meeting = library.LastMeeting;
        var shown = meeting is not null;
        LastMeetingTitleLink.Visibility = Show(shown);
        LastMeetingLine.Visibility = Show(shown);
        LastMeetingButtons.Visibility = Show(shown);
        LastMeetingLede.Inlines.Clear();
        LastMeetingLede.Visibility = Visibility.Collapsed;
        if (meeting is not null)
        {
            var title = LibraryFormat.Title(meeting.Record);
            LastMeetingTitle.Text = title;
            AutomationProperties.SetName(LastMeetingTitleLink, title);
            LastMeetingLine.Text = LibraryFormat.HeaderLine(meeting.Record, meeting.People, now, calendar);
            if (meeting.Summary?.Lede is { } lede)
            {
                foreach (var run in lede.Runs)
                {
                    LastMeetingLede.Inlines.Add(Inline(run));
                }
                LastMeetingLede.Visibility = Visibility.Visible;
            }
            PlayButton.Visibility = Show(meeting.Record.HasAudio);
            var play = TodayText.PlayTitle(TodayText.PlayFromMs(meeting));
            PlayText.Text = play;
            AutomationProperties.SetName(PlayButton, play);
        }
        var note = TodayText.LastMeetingNote(
            shown, library.LastMeetingLoad == LibraryLoad.Loaded, library.LastMeetingLoad == LibraryLoad.Failed);
        LastMeetingNote.Text = note ?? "";
        LastMeetingNote.Visibility = Show(note is not null);
        LastMeetingRetry.Visibility = Show(!shown && library.LastMeetingLoad == LibraryLoad.Failed);
    }

    private static Run Inline(SummaryRun run)
    {
        var inline = new Run { Text = run.Text };
        if (run.Bold)
        {
            inline.FontWeight = FontWeights.SemiBold;
        }
        if (run.Italic)
        {
            inline.FontStyle = Windows.UI.Text.FontStyle.Italic;
        }
        if (run.Code)
        {
            inline.FontFamily = (FontFamily)Application.Current.Resources["InkMonoFontFamily"];
        }
        if (run.Strikethrough)
        {
            inline.TextDecorations = Windows.UI.Text.TextDecorations.Strikethrough;
        }
        return inline;
    }

    private void OnOpenLastMeeting(object sender, RoutedEventArgs e)
    {
        if (library.LastMeeting is { } meeting)
        {
            openRecord(meeting.Record.Record, null, false);
        }
    }

    private void OnPlayLastMeeting(object sender, RoutedEventArgs e)
    {
        if (library.LastMeeting is { } meeting)
        {
            openRecord(meeting.Record.Record, TodayText.PlayFromMs(meeting), true);
        }
    }

    private void OnRetryToday(object sender, RoutedEventArgs e) => library.RefreshToday();

    // Up next

    private void RenderUpNext(DateTimeOffset now, LibraryCalendar calendar)
    {
        var shown = upNext.Event;
        EventTitle.Text = shown?.Title ?? "";
        EventTitle.Visibility = Show(shown is not null);
        EventLine.Text = upNext.MetaLine(now, calendar) ?? "";
        EventLine.Visibility = Show(shown is not null);
        EventRecords.Text = upNext.RecordsWhen ?? "";
        EventRecords.Visibility = Show(shown is not null);
        UpNextNote.Text = upNext.Note ?? "";
        UpNextNote.Visibility = Show(upNext.Note is not null);
        ConnectButton.Content = upNext.ConnectTitle;
        ConnectButton.Visibility = Show(upNext.ConnectTitle is not null);
        ScheduleMinute(now);
    }

    /// <summary>The next redraw of "in N min": at the next whole minute, only while it can change and be seen.</summary>
    private void ScheduleMinute(DateTimeOffset now)
    {
        minute.Stop();
        if (loaded && upNext.Ticks(presence.OnScreen))
        {
            minute.Interval = MinuteSchedule.NextMinute(now) - now;
            minute.Start();
        }
    }

    private void MinuteTick()
    {
        var now = library.Now();
        EventLine.Text = upNext.MetaLine(now, library.Calendar) ?? "";
        ScheduleMinute(now);
    }

    private void OnConnect(object sender, RoutedEventArgs e) => upNext.Connect();

    // Owed soon

    private void RenderOwed(DateTimeOffset now)
    {
        var all = TodayText.OwedAllTitle(owed.Items.Count);
        OwedAll.Content = all;
        OwedAll.Visibility = Show(all is not null);
        var failure = owed.LoadFailure ?? owed.Failure;
        var note = TodayText.OwedNote(owed.Loaded, owed.Items.Count, failure);
        OwedNote.Text = note ?? "";
        OwedNote.Visibility = Show(note is not null);
        OwedNote.Foreground = BrushOf(failure is not null ? "InkAlertBrush" : "InkSecondaryTextBrush");
        OwedRows.Children.Clear();
        foreach (var row in TodayText.OwedSoonRows(owed, now))
        {
            OwedRows.Children.Add(OwedRow(row));
        }
    }

    private Grid OwedRow(OwedSoonRow row)
    {
        var line = new Grid { ColumnSpacing = 12 };
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var circle = new Button
        {
            Width = 20,
            Height = 20,
            Padding = new Thickness(0),
            Margin = new Thickness(0, 1, 0, 0),
            MinWidth = 0,
            MinHeight = 0,
            VerticalAlignment = VerticalAlignment.Top,
            CornerRadius = new CornerRadius(10),
            BorderThickness = new Thickness(1.5),
            BorderBrush = BrushOf("InkTextBrush"),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
        };
        AutomationProperties.SetName(circle, row.DoneLabel);
        circle.Click += (_, _) => owed.MarkDone(row.Id);
        line.Children.Add(circle);
        var words = new StackPanel { Spacing = 3 };
        words.Children.Add(new TextBlock { Text = row.Text, FontSize = 14.5, Style = StyleOf("InkBodyStyle") });
        var meta = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        meta.Children.Add(new TextBlock
        {
            Text = row.Meta,
            Style = StyleOf("InkTimestampStyle"),
            FontSize = 11,
            Foreground = BrushOf(row.Overdue ? "InkAlertBrush" : "InkSecondaryTextBrush"),
            TextTrimming = TextTrimming.CharacterEllipsis,
        });
        if (row.SaidAtMs is long at && row.PlayTitle is { } play)
        {
            var link = new HyperlinkButton
            {
                Content = play,
                Padding = new Thickness(0),
                FontFamily = (FontFamily)Application.Current.Resources["InkMonoFontFamily"],
                FontSize = 11,
            };
            AutomationProperties.SetName(link, row.PlayLabel);
            link.Click += (_, _) => openRecord(row.Record, at, true);
            meta.Children.Add(link);
        }
        words.Children.Add(meta);
        Grid.SetColumn(words, 1);
        line.Children.Add(words);
        return line;
    }

    private void OnOpenOwed(object sender, RoutedEventArgs e) => open(Route.Owed);

    // Stats

    private void RenderStats(LibraryCalendar calendar)
    {
        Stats.Children.Clear();
        var lines = TodayText.StatsLines(
            library.Today, library.TodayLoad == LibraryLoad.Failed, library.Week, library.WeekLoad == LibraryLoad.Failed, calendar.Culture);
        foreach (var line in lines)
        {
            Stats.Children.Add(new TextBlock { Text = line, Style = StyleOf("InkTimestampStyle"), FontSize = 11.5 });
        }
    }

    // Layout

    /// <summary>Two columns when the content is wide enough, else one; the counts at the foot of the visible height.</summary>
    private void Layout()
    {
        var width = ActualWidth - Gutters;
        var wide = width >= TwoColumns;
        Grid.SetColumnSpan(LastMeeting, wide ? 1 : 2);
        Grid.SetColumn(RightColumn, wide ? 1 : 0);
        Grid.SetColumnSpan(RightColumn, wide ? 1 : 2);
        Grid.SetRow(RightColumn, wide ? 0 : 1);
        Stats.Orientation = wide ? Orientation.Horizontal : Orientation.Vertical;
        Stats.Spacing = wide ? 28 : 4;
        Page.MinHeight = Scroller.ViewportHeight;
    }

    private static Visibility Show(bool visible) => visible ? Visibility.Visible : Visibility.Collapsed;

    private static Brush BrushOf(string key) => (Brush)Application.Current.Resources[key];

    private static Style StyleOf(string key) => (Style)Application.Current.Resources[key];

    /// <summary>Opens Windows' Sound settings; one that does not open is logged by name.</summary>
    private static async void OpenSoundSettings()
    {
        try
        {
            if (!await Windows.System.Launcher.LaunchUriAsync(new Uri("ms-settings:sound")))
            {
                ScreenLog.System.Write("Sound settings did not open");
            }
        }
        catch (Exception e)
        {
            ScreenLog.System.Write($"Sound settings could not be opened ({e.GetType().Name})");
        }
    }
}
