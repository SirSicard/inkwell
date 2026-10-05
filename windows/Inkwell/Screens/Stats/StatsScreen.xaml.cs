// The Stats screen's view: asks again each time it shows (StatsModel.ScreenAppeared) and while the
// window comes back on screen, and builds its four cards from the model's answer. The words are
// StatsFormat's; the cards are built in code, rebuilt on each answer and when the mode or the
// colours change. Narrator reads one name per number, one sentence for the heatmap and the talk
// bar, and each milestone chip as "…, reached" or "…, not yet".
using System.ComponentModel;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;

namespace Inkwell.Screens;

public sealed partial class StatsScreen : UserControl
{
    /// <summary>The page's widest, padding included (the Mac's 860 points of cards).</summary>
    private const double PageWidth = 956;

    /// <summary>A heatmap day's square and the gap between them (the Mac's 12 and 3).</summary>
    private const double CellSize = 12;
    private const double CellGap = 3;

    /// <summary>A heatmap day's shade of your colour by its level.</summary>
    private static readonly double[] LevelOpacity = [0, 0.3, 0.5, 0.75, 1];

    private readonly StatsModel stats;
    private readonly GlowTheme theme;
    private readonly WindowPresence presence;
    private readonly Func<nint> windowHandle;

    /// <param name="windowHandle">The main window, which the share card's save picker belongs to.</param>
    internal StatsScreen(StatsModel stats, GlowTheme theme, WindowPresence presence, Func<nint> windowHandle)
    {
        ArgumentNullException.ThrowIfNull(stats);
        ArgumentNullException.ThrowIfNull(theme);
        ArgumentNullException.ThrowIfNull(presence);
        ArgumentNullException.ThrowIfNull(windowHandle);
        this.stats = stats;
        this.theme = theme;
        this.presence = presence;
        this.windowHandle = windowHandle;
        InitializeComponent();
        Loaded += (_, _) =>
        {
            stats.PropertyChanged += OnStatsChanged;
            presence.PropertyChanged += OnPresenceChanged;
            theme.Changed += Render;
            stats.ScreenAppeared();
            // Off screen it waits; back on screen it counts again.
            stats.WindowPresence(presence.OnScreen);
            Render();
        };
        Unloaded += (_, _) =>
        {
            stats.PropertyChanged -= OnStatsChanged;
            presence.PropertyChanged -= OnPresenceChanged;
            theme.Changed -= Render;
            stats.ScreenDisappeared();
        };
        ActualThemeChanged += (_, _) => Render();
    }

    private void OnStatsChanged(object? sender, PropertyChangedEventArgs e) => Render();

    private void OnPresenceChanged(object? sender, PropertyChangedEventArgs e) => stats.WindowPresence(presence.OnScreen);

    private void OnScrollerSized(object sender, SizeChangedEventArgs e) => Page.Width = Math.Min(e.NewSize.Width, PageWidth);

    private void OnTryAgain(object sender, RoutedEventArgs e) => stats.Reload();

    private async void OnShare(object sender, RoutedEventArgs e)
    {
        if (stats.Counted is null || XamlRoot is null)
        {
            return;
        }
        try
        {
            await new StatsShareDialog(stats, theme, windowHandle, XamlRoot, ActualTheme).ShowAsync();
        }
        catch (Exception failure)
        {
            // Another dialog is open (only one can be: a second click while it opens, say), the card
            // could not be made, or it failed while open: nothing more is shared, and the app goes on.
            ScreenLog.System.Write($"the share card failed ({failure.GetType().Name})");
        }
    }

    private void Render()
    {
        var counted = stats.Counted;
        var failed = stats.LoadState == StatsModel.Load.Failed;
        ShareButton.Visibility = Shown(counted is not null);
        StaleLine.Visibility = Shown(counted is not null && failed);
        FailedRow.Visibility = Shown(counted is null && failed);
        LoadingRing.IsActive = counted is null && !failed;
        LoadingRing.Visibility = Shown(counted is null && !failed);
        Cards.Children.Clear();
        if (counted is null)
        {
            return;
        }
        Cards.Children.Add(DictationCard(counted));
        Cards.Children.Add(MeetingsCard(counted));
        Cards.Children.Add(PromisesCard(counted));
        Cards.Children.Add(MilestonesCard(counted.Milestones));
    }

    private static Visibility Shown(bool shown) => shown ? Visibility.Visible : Visibility.Collapsed;

    // Parts

    /// <summary>A card's frame: the eyebrow over the content, Glow's translucent card.</summary>
    private static Border Card(string title, IEnumerable<UIElement> content)
    {
        var body = new StackPanel { Spacing = 12 };
        body.Children.Add(Parts.Eyebrow(title));
        foreach (var part in content)
        {
            body.Children.Add(part);
        }
        return new Border { Style = (Style)Application.Current.Resources["InkCardStyle"], Child = body };
    }

    /// <summary>A line of the body text that wraps; <paramref name="secondary"/>, in the caption's grey.</summary>
    private static TextBlock Line(string text, bool secondary = false) => Parts.Text(text, secondary ? "InkCaptionStyle" : "InkBodyStyle");

    /// <summary>
    /// A number over its label, read as one: "1,234 words today". One text of two runs, not two
    /// texts with a name on the number: Narrator's scan mode reads a text's words, not its name,
    /// so the number was read alone ("38, 38, 38").
    /// </summary>
    private TextBlock BigNumber(string value, string label)
    {
        var text = Parts.Text("", "InkCaptionStyle");
        text.LineHeight = 0;
        text.Inlines.Add(new Run
        {
            Text = value,
            FontFamily = Parts.Font("InkDisplayFontFamily"),
            FontSize = (double)Application.Current.Resources["GlowHeadingFontSize"],
            Foreground = Parts.Brush("InkTextBrush", this),
        });
        text.Inlines.Add(new LineBreak());
        text.Inlines.Add(new Run { Text = label });
        return text;
    }

    /// <summary>Numbers side by side when they fit, one under another when not.</summary>
    private WrapPanel NumberRow(params (string Value, string Label)[] numbers)
    {
        var row = new WrapPanel { Spacing = 32, RowSpacing = 10 };
        foreach (var (value, label) in numbers)
        {
            row.Children.Add(BigNumber(value, label));
        }
        return row;
    }

    private Border DictationCard(StatsCounted counted)
    {
        var d = counted.Dictation;
        var culture = stats.Culture;
        var parts = new List<UIElement>();
        if (d.DictationsAll == 0)
        {
            parts.Add(Line("Your words, speed and streak appear here after your first dictation.", secondary: true));
        }
        else
        {
            parts.Add(NumberRow(
                (StatsFormat.Count(d.WordsToday, culture), "words today"),
                (StatsFormat.Count(d.WordsWeek, culture), "this week"),
                (StatsFormat.Count(d.WordsAll, culture), "all time")));
            parts.Add(Line(StatsFormat.Speed(d.WpmWeek, d.WpmAverage)));
            parts.Add(Line(StatsFormat.Saved(d.SavedMsAll, counted.TypingWpm)));
            if (d.SavedMsWeek > 0)
            {
                parts.Add(Line($"This week: {LibraryFormat.Duration(d.SavedMsWeek)}", secondary: true));
            }
            if (StatsFormat.Streak(d.StreakDays, d.LongestStreakDays) is string streak)
            {
                parts.Add(Line(streak));
                parts.Add(Line(StatsFormat.StreakRule, secondary: true));
            }
        }
        parts.Add(Heatmap(StatsFormat.Heatmap(d)));
        return Card("Dictation", parts);
    }

    /// <summary>
    /// Words per day over the last 12 weeks: a column per week, a row per weekday, shaded in your
    /// colour by words against the busiest day. Narrator reads one sentence for it.
    /// </summary>
    private StackPanel Heatmap(IReadOnlyList<StatsFormat.Cell> cells)
    {
        var weeks = cells.Count == 0 ? 0 : cells[^1].Week + 1;
        var grid = new Grid { ColumnSpacing = CellGap, RowSpacing = CellGap, HorizontalAlignment = HorizontalAlignment.Left };
        for (var w = 0; w < weeks; w++)
        {
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(CellSize) });
        }
        for (var r = 0; r < 7; r++)
        {
            grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(CellSize) });
        }
        var chip = Parts.Brush("InkChipBrush", this);
        foreach (var cell in cells)
        {
            // A day not yet come (after today in this week) is left out: there is no cell for it.
            var square = new Rectangle
            {
                Width = CellSize,
                Height = CellSize,
                RadiusX = 3,
                RadiusY = 3,
                Fill = cell.Level == 0 ? chip : new SolidColorBrush(GlowTheme.ColorOf(theme.Colours.You, (byte)Math.Round(LevelOpacity[cell.Level] * 255))),
            };
            Grid.SetColumn(square, cell.Week);
            Grid.SetRow(square, cell.Weekday);
            grid.Children.Add(square);
        }
        var caption = Parts.Text("Last 12 weeks", "InkCaptionStyle");
        AutomationProperties.SetName(caption, StatsFormat.HeatmapSummary(cells, stats.Culture));
        var panel = new StackPanel { Spacing = 6 };
        panel.Children.Add(grid);
        panel.Children.Add(caption);
        return panel;
    }

    private Border MeetingsCard(StatsCounted counted)
    {
        var month = counted.MeetingsMonth;
        var all = counted.MeetingsAll;
        var parts = new List<UIElement>();
        if (all.Meetings == 0)
        {
            parts.Add(Line("Hours, talk time and the questions you asked appear here after your first recorded meeting.", secondary: true));
        }
        else
        {
            if (month.Meetings == 0)
            {
                parts.Add(Line("No meetings recorded this month yet."));
            }
            else
            {
                parts.Add(NumberRow(
                    (LibraryFormat.Duration(month.RecordedMs), "recorded"),
                    ($"{month.Meetings}", month.Meetings == 1 ? "meeting" : "meetings"),
                    ($"{month.Questions}", month.Questions == 1 ? "question you asked" : "questions you asked")));
                parts.Add(TalkTime(month.YouMs, month.ThemMs));
                parts.Add(Line($"Your longest monologue: {StatsFormat.Span(month.LongestMonologueMs)}"));
                parts.Add(Line("Questions are your own lines that end in a question mark.", secondary: true));
            }
            parts.Add(Line($"All time: {all.Meetings} {(all.Meetings == 1 ? "meeting" : "meetings")} · {LibraryFormat.Duration(all.RecordedMs)} recorded", secondary: true));
        }
        return Card("Meetings this month", parts);
    }

    /// <summary>You and them, as a bar in your two colours and in words. Exact: your microphone is you, the call's sound is them.</summary>
    private UIElement TalkTime(long you, long them)
    {
        if (StatsFormat.TalkShare(you, them) is not (int, int) share)
        {
            return Line("No talk time recorded this month.", secondary: true);
        }
        var youBrush = new SolidColorBrush(GlowTheme.ColorOf(theme.Colours.You));
        var themBrush = new SolidColorBrush(GlowTheme.ColorOf(theme.Colours.Them));
        var bar = new Grid { Height = 8, ColumnSpacing = 2 };
        bar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(Math.Max(share.You, 0.001), GridUnitType.Star) });
        bar.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(Math.Max(share.Them, 0.001), GridUnitType.Star) });
        var mine = new Border { Background = youBrush, CornerRadius = new CornerRadius(4) };
        var theirs = new Border { Background = themBrush, CornerRadius = new CornerRadius(4) };
        Grid.SetColumn(theirs, 1);
        bar.Children.Add(mine);
        bar.Children.Add(theirs);
        // The key: a dot in each colour before its words (no words in the dot colours).
        var key = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 14 };
        var youKey = Key(youBrush, StatsFormat.TalkKey("You", share.You, you));
        var themKey = Key(themBrush, StatsFormat.TalkKey("Them", share.Them, them));
        key.Children.Add(youKey.Panel);
        key.Children.Add(themKey.Panel);
        // One sentence for the bar and its key.
        AutomationProperties.SetName(youKey.Words, StatsFormat.TalkSpoken(share, you, them));
        AutomationProperties.SetAccessibilityView(themKey.Words, AccessibilityView.Raw);
        var panel = new StackPanel { Spacing = 6 };
        panel.Children.Add(bar);
        panel.Children.Add(key);
        return panel;
    }

    private static (StackPanel Panel, TextBlock Words) Key(Brush colour, string words)
    {
        var panel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
        panel.Children.Add(new Ellipse { Width = 8, Height = 8, Fill = colour, VerticalAlignment = VerticalAlignment.Center });
        var text = Parts.Text(words, "InkCaptionStyle");
        panel.Children.Add(text);
        return (panel, text);
    }

    private static Border PromisesCard(StatsCounted counted)
    {
        var month = counted.PromisesMonth;
        var all = counted.PromisesAll;
        var parts = new List<UIElement>();
        if (all.Made == 0)
        {
            parts.Add(Line("Promises from your meetings, kept and open, appear here once a meeting has some.", secondary: true));
        }
        else
        {
            if (month.Made == 0)
            {
                parts.Add(Line("No promises made this month."));
            }
            else
            {
                parts.Add(Parts.Text(StatsFormat.Kept(month, "this month"), "InkHeadingStyle"));
                parts.Add(Line(StatsFormat.PromiseDetail(month)));
            }
            parts.Add(Line($"All time: kept {all.Kept} of {all.Made} · " + StatsFormat.PromiseDetail(all), secondary: true));
        }
        return Card("Promises", parts);
    }

    private Border MilestonesCard(IReadOnlyList<MilestoneRow> milestones)
    {
        var chips = new WrapPanel { Spacing = 8, RowSpacing = 8 };
        foreach (var m in milestones)
        {
            var title = StatsFormat.MilestoneTitle(m.Kind, m.Threshold, stats.Culture);
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
            // Segoe Fluent Icons: CompletedSolid for reached, CircleRing for not yet.
            row.Children.Add(new FontIcon
            {
                Glyph = m.Reached ? "" : "",
                FontSize = 12,
                Foreground = Parts.Brush(m.Reached ? "InkTextBrush" : "InkSecondaryTextBrush", this),
                VerticalAlignment = VerticalAlignment.Center,
            });
            var words = new TextBlock
            {
                Text = title,
                FontSize = 12,
                FontWeight = m.Reached ? FontWeights.Medium : FontWeights.Normal,
                Foreground = Parts.Brush(m.Reached ? "InkTextBrush" : "InkSecondaryTextBrush", this),
                VerticalAlignment = VerticalAlignment.Center,
            };
            AutomationProperties.SetName(words, StatsFormat.MilestoneSpoken(m, stats.Culture));
            row.Children.Add(words);
            chips.Children.Add(new Border
            {
                Child = row,
                Padding = new Thickness(10, 4, 10, 4),
                CornerRadius = new CornerRadius(999),
                Background = m.Reached ? Parts.Brush("InkChipBrush", this) : null,
                BorderBrush = Parts.Brush("InkBorderBrush", this),
                BorderThickness = new Thickness(m.Reached ? 0 : 1),
            });
        }
        return Card("Milestones", [chips]);
    }
}
