// How the Stats screen's numbers read, as the Mac's StatsFormat. Pure: the caller passes the
// culture, so the tests pin it. Every figure keeps its assumption beside it (time saved names the
// typing speed), and speed is only ever against the user's own past.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public static class StatsFormat
{
    /// <summary>A count in the user's culture: <c>12,345</c>.</summary>
    public static string Count(long n, CultureInfo? culture = null) => n.ToString("N0", culture ?? CultureInfo.CurrentCulture);

    /// <summary>
    /// Time saved, always with the speed it assumes. None (or less than none: slow, paused
    /// dictation) is said as none yet, never as a negative time.
    /// </summary>
    public static string Saved(long ms, long typingWpm) => ms > 0
        ? $"{LibraryFormat.Duration(ms)} saved vs typing at {typingWpm} wpm"
        : $"No time saved yet vs typing at {typingWpm} wpm";

    /// <summary>Words per minute this week against the user's own 30-day average.</summary>
    public static string Speed(long? week, long? average) => (week, average) switch
    {
        (long w, long a) => $"{w} wpm this week · your 30-day average {a} wpm",
        (long w, null) => $"{w} wpm this week",
        (null, long a) => $"No speed this week yet · your 30-day average {a} wpm",
        _ => "Your speed shows after a minute of dictation",
    };

    /// <summary>
    /// The streak, from its second day; before that the longest, when there was one. Always in
    /// active days: one missed day between two doesn't end a streak, so it is not calendar days.
    /// </summary>
    public static string? Streak(long current, long longest)
    {
        if (current >= 2)
        {
            return current == longest
                ? $"Streak: {current} active days"
                : $"Streak: {current} active days · longest {longest}";
        }
        return longest >= 2 ? $"Longest streak: {longest} active days" : null;
    }

    /// <summary>What a streak counts, said beside it.</summary>
    public const string StreakRule = "Active days are days you dictated. One missed day between them doesn't end a streak.";

    /// <summary>A short span with its seconds: <c>45 s</c>, <c>4 min 10 s</c>, <c>1 h 5 min</c>.</summary>
    public static string Span(long ms)
    {
        var seconds = Math.Max(ms, 0) / 1000;
        if (seconds < 60)
        {
            return $"{seconds} s";
        }
        if (seconds < 3600)
        {
            var (m, s) = (seconds / 60, seconds % 60);
            return s == 0 ? $"{m} min" : $"{m} min {s} s";
        }
        var (h, min) = (seconds / 3600, seconds / 60 % 60);
        return min == 0 ? $"{h} h" : $"{h} h {min} min";
    }

    /// <summary>Talk time as whole percentages that add up to 100; null when nobody spoke.</summary>
    public static (int You, int Them)? TalkShare(long you, long them)
    {
        var total = you + them;
        if (total <= 0)
        {
            return null;
        }
        // Rounded half away from zero, as Swift's rounded().
        var mine = (int)Math.Round(you * 100.0 / total, MidpointRounding.AwayFromZero);
        return (mine, 100 - mine);
    }

    /// <summary><c>Kept 12 of 14 this month</c>.</summary>
    public static string Kept(PromiseStats p, string period)
    {
        ArgumentNullException.ThrowIfNull(p);
        return $"Kept {p.Kept} of {p.Made} {period}";
    }

    /// <summary><c>1 open · 1 overdue</c>.</summary>
    public static string PromiseDetail(PromiseStats p)
    {
        ArgumentNullException.ThrowIfNull(p);
        return $"{p.Open} open · {p.Overdue} overdue";
    }

    /// <summary>A milestone's name: <c>1,000 words</c>, <c>7 active days in a row</c>.</summary>
    public static string MilestoneTitle(MilestoneKind kind, long threshold, CultureInfo? culture = null) => kind switch
    {
        MilestoneKind.Words => $"{Count(threshold, culture)} words",
        _ => $"{threshold} active days in a row",
    };

    /// <summary>The one line a milestone reached gets.</summary>
    public static string MilestoneNote(MilestoneKind kind, long threshold, CultureInfo? culture = null) => kind switch
    {
        MilestoneKind.Words => $"Milestone · {Count(threshold, culture)} words dictated",
        _ => $"Milestone · {threshold} active days in a row",
    };

    /// <summary>A milestone chip's name for Narrator.</summary>
    public static string MilestoneSpoken(MilestoneRow m, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(m);
        return $"{MilestoneTitle(m.Kind, m.Threshold, culture)}, {(m.Reached ? "reached" : "not yet")}";
    }

    /// <summary>The talk bar's key, one side: <c>You 60 % · 4 min</c>.</summary>
    public static string TalkKey(string side, int percent, long ms) => $"{side} {percent} % · {Span(ms)}";

    /// <summary>What Narrator hears for the talk bar and its key.</summary>
    public static string TalkSpoken((int You, int Them) share, long you, long them) =>
        $"Talk time: you {share.You} percent, {Span(you)}; them {share.Them} percent, {Span(them)}";

    // Heatmap

    /// <summary>One day of the heatmap.</summary>
    /// <param name="Index">Its place: days from the map's first.</param>
    /// <param name="Level">0 for none, then 1 to 4 against the busiest day shown.</param>
    public sealed record Cell(int Index, DateOnly Date, long Words, int Level)
    {
        /// <summary>Its row: days from the first day of its week (the map's columns are weeks).</summary>
        public int Weekday => Index % 7;

        /// <summary>Its column.</summary>
        public int Week => Index / 7;
    }

    /// <summary>The heatmap's days, shaded by words against the busiest one.</summary>
    public static IReadOnlyList<Cell> Heatmap(DictationStats d)
    {
        ArgumentNullException.ThrowIfNull(d);
        if (Day(d.HeatmapFirstDay) is not DateOnly first)
        {
            return [];
        }
        var busiest = d.HeatmapWords.Count == 0 ? 0 : d.HeatmapWords.Max();
        return d.HeatmapWords.Select((words, index) =>
        {
            // Quarters of the busiest day: a quiet day still shows, faintly.
            var level = words <= 0 || busiest <= 0 ? 0 : Math.Min(4, Math.Max(1, (int)Math.Ceiling(words * 4.0 / busiest)));
            return new Cell(index, first.AddDays(index), words, level);
        }).ToList();
    }

    /// <summary>What Narrator hears for the heatmap: one sentence, not a cell per day.</summary>
    public static string HeatmapSummary(IReadOnlyList<Cell> cells, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(cells);
        culture ??= CultureInfo.CurrentCulture;
        const string Lead = "Words dictated per day over the last 12 weeks: ";
        var active = cells.Where(c => c.Words > 0).ToList();
        if (active.Count == 0)
        {
            return Lead + "no dictation yet.";
        }
        // The first of the busiest days, as Swift's max(by:) keeps the first of equals.
        var most = active.Aggregate((a, b) => b.Words > a.Words ? b : a);
        return Lead + $"{active.Count} of {cells.Count} days with dictation. "
            + $"The most was {Count(most.Words, culture)} words, on {most.Date.ToString("ddd d MMM", culture)}.";
    }

    /// <summary><c>YYYY-MM-DD</c> as that day.</summary>
    public static DateOnly? Day(string text) =>
        DateOnly.TryParseExact(text, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out var day) ? day : null;
}
