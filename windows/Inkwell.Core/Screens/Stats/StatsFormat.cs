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

    /// <summary>
    /// The streak as the Dictation card says it: running, or once it has ended, by what it reached
    /// and the longest, never as lost (the core keeps the latest). Nothing while it is hidden.
    /// </summary>
    public static string? Streak(DictationStats d)
    {
        ArgumentNullException.ThrowIfNull(d);
        if (d.StreakHidden == true)
        {
            return null;
        }
        if (d.StreakDays < 2 && d.LatestStreakDays is long latest && latest >= 2 && latest < d.LongestStreakDays)
        {
            return $"Latest streak: {latest} active days · longest {d.LongestStreakDays}";
        }
        return Streak(d.StreakDays, d.LongestStreakDays);
    }

    /// <summary>What a streak counts, said beside it.</summary>
    public const string StreakRule = "Active days are days you dictated. One missed day between them doesn't end a streak.";

    /// <summary>The rule, with the rest days when there are some: <c>Rest days (Sat, Sun) neither count nor break it.</c></summary>
    public static string StreakRuleWith(IReadOnlyList<long>? restDays, CultureInfo? culture = null)
    {
        var names = Weekdays(culture).Where(w => restDays?.Contains(w.Iso) ?? false).Select(w => w.Abbreviated).ToList();
        return names.Count == 0 ? StreakRule : $"{StreakRule} Rest days ({string.Join(", ", names)}) neither count nor break it.";
    }

    /// <summary>A pause running since <paramref name="since"/> (YYYY-MM-DD).</summary>
    public static string Paused(string since, CultureInfo? culture = null) =>
        $"Paused since {ShortDate(since, culture)}: days without a dictation don't count against it.";

    /// <summary>Once no streak is running: the days dictated this month (<c>14 active days this month</c>).</summary>
    public static string? ActiveDaysThisMonth(DictationStats d)
    {
        ArgumentNullException.ThrowIfNull(d);
        if (d.StreakHidden == true || d.StreakDays >= 2 || d.ActiveDaysMonth is not (> 0 and var days))
        {
            return null;
        }
        return days == 1 ? "1 active day this month" : $"{days} active days this month";
    }

    /// <summary>A weekday as Settings shows it: its ISO number (1 Monday to 7 Sunday), short and full name.</summary>
    public sealed record Weekday(int Iso, string Abbreviated, string Name);

    /// <summary>The seven weekdays in the order the user's week runs, named in their culture.</summary>
    public static IReadOnlyList<Weekday> Weekdays(CultureInfo? culture = null)
    {
        culture ??= CultureInfo.CurrentCulture;
        var start = StatsModel.IsoWeekStart(culture);
        return Enumerable.Range(0, 7).Select(offset =>
        {
            var iso = (start - 1 + offset) % 7 + 1;
            // .NET counts Sunday as 0; ISO counts Monday as 1.
            var day = iso % 7;
            return new Weekday(iso, culture.DateTimeFormat.AbbreviatedDayNames[day], culture.DateTimeFormat.DayNames[day]);
        }).ToList();
    }

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

    /// <summary>A milestone's count: <c>1,000 words</c>, <c>7 active days in a row</c>.</summary>
    public static string MilestoneTitle(MilestoneKind kind, long threshold, CultureInfo? culture = null) => kind switch
    {
        MilestoneKind.Words => $"{Count(threshold, culture)} words",
        _ => $"{threshold} active days in a row",
    };

    /// <summary>A milestone's name (the core's key), the same on the chip, in the celebration and on the share card's seal.</summary>
    public static string MilestoneNamed(MilestoneName name) => name switch
    {
        MilestoneName.FirstPage => "First page",
        MilestoneName.Notebook => "A notebook",
        MilestoneName.ShortNovel => "A short novel",
        MilestoneName.NovelsWorth => "A novel's worth",
        MilestoneName.SevenDays => "Seven-day run",
        MilestoneName.ThirtyDays => "Thirty-day run",
        MilestoneName.HundredDays => "Hundred-day run",
        _ => throw new ArgumentOutOfRangeException(nameof(name)),
    };

    /// <summary>A milestone's chip: its name and its count, <c>First page · 1,000 words</c>.</summary>
    public static string MilestoneChip(MilestoneRow m, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(m);
        var title = MilestoneTitle(m.Kind, m.Threshold, culture);
        return m.Name is { } name ? $"{MilestoneNamed(name)} · {title}" : title;
    }

    /// <summary>The one line a milestone reached gets.</summary>
    public static string MilestoneNote(MilestoneKind kind, long threshold, CultureInfo? culture = null, MilestoneName? name = null)
    {
        var what = kind == MilestoneKind.Words ? $"{Count(threshold, culture)} words dictated" : $"{threshold} active days in a row";
        return $"{(name is { } n ? MilestoneNamed(n) : "Milestone")} · {what}";
    }

    /// <summary>A seal's mark: the count, short (<c>1k</c>, <c>100k</c>, <c>30</c>).</summary>
    public static string SealMark(MilestoneKind kind, long threshold) =>
        kind == MilestoneKind.Words && threshold >= 1000 && threshold % 1000 == 0
            ? string.Create(CultureInfo.InvariantCulture, $"{threshold / 1000}k")
            : threshold.ToString(CultureInfo.InvariantCulture);

    /// <summary>A milestone chip's name for Narrator.</summary>
    public static string MilestoneSpoken(MilestoneRow m, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(m);
        return $"{MilestoneChip(m, culture)}, {(m.Reached ? "reached" : "not yet")}";
    }

    // Time saved, pictured

    /// <summary>
    /// What time saved is about, in words, from the core's largest picture that fits:
    /// <c>about 2 feature films</c>. Null when the core has none (too little time, or between counts).
    /// </summary>
    public static string? About(IReadOnlyList<TimeEquivalent>? equivalents) =>
        equivalents is { Count: > 0 } && equivalents[0] is { Count: >= 1 } first ? "about " + Equivalent(first.Key, first.Count) : null;

    /// <summary>One of the core's pictures of time: <c>a lunch hour</c>, <c>3 working days</c>.</summary>
    public static string Equivalent(TimeEquivalentKey key, long count)
    {
        var (one, many) = key switch
        {
            TimeEquivalentKey.WorkingWeek => ("a working week", "working weeks"),
            TimeEquivalentKey.WorkingDay => ("a working day", "working days"),
            TimeEquivalentKey.FeatureFilm => ("a feature film", "feature films"),
            TimeEquivalentKey.LunchHour => ("a lunch hour", "lunch hours"),
            TimeEquivalentKey.CoffeeBreak => ("a coffee break", "coffee breaks"),
            _ => throw new ArgumentOutOfRangeException(nameof(key)),
        };
        return count == 1 ? one : string.Create(CultureInfo.InvariantCulture, $"{count} {many}");
    }

    /// <summary>The line under time saved: <c>That's about 2 working days.</c></summary>
    public static string? SavedAbout(IReadOnlyList<TimeEquivalent>? equivalents) => About(equivalents) is string about ? $"That's {about}." : null;

    /// <summary>This week's time saved: <c>This week: 25 min, about 2 coffee breaks</c>.</summary>
    public static string SavedThisWeek(long ms, IReadOnlyList<TimeEquivalent>? equivalents) =>
        $"This week: {LibraryFormat.Duration(ms)}" + (About(equivalents) is string about ? $", {about}" : "");

    // Records

    /// <summary>A best on the Records card: its value, what it is and when, and what Narrator hears.</summary>
    public sealed record Record(BestId Id, string Value, string Label, string Spoken);

    /// <summary>What a best is, in a line: <c>longest dictation</c>.</summary>
    public static string BestName(BestId id) => id switch
    {
        BestId.LongestDictation => "longest dictation",
        BestId.FastestDictation => "fastest dictation",
        BestId.MostWordsDay => "most words in a day",
        BestId.BestWeek => "most words in a week",
        BestId.LongestMeeting => "longest meeting",
        BestId.LongestMonologue => "longest monologue",
        _ => throw new ArgumentOutOfRangeException(nameof(id)),
    };

    /// <summary>A best's value in its unit.</summary>
    public static string BestValue(long value, BestUnit unit, CultureInfo? culture = null) => unit switch
    {
        BestUnit.Ms => Span(value),
        BestUnit.Wpm => string.Create(CultureInfo.InvariantCulture, $"{value} wpm"),
        _ => $"{Count(value, culture)} words",
    };

    /// <summary>When a best was set: its day (<c>4 Oct</c>), or for the best week, <c>week of 28 Sep</c>.</summary>
    public static string BestWhen(BestId id, string date, CultureInfo? culture = null) =>
        id == BestId.BestWeek ? $"week of {ShortDate(date, culture)}" : ShortDate(date, culture);

    /// <summary>The Records card's rows, in the core's order; none for a best not held yet.</summary>
    public static IReadOnlyList<Record> Records(IReadOnlyList<BestRow>? bests, CultureInfo? culture = null) =>
        (bests ?? []).Select(b =>
        {
            var value = BestValue(b.Value, b.Unit, culture);
            var when = BestWhen(b.Id, b.Date, culture);
            var name = BestName(b.Id);
            return new Record(b.Id, value, $"{name} · {when}", $"{Capitalised(name)}: {value}, {(b.Id == BestId.BestWeek ? "the " : "on ")}{when}");
        }).ToList();

    /// <summary>What the Records card counts, said under it.</summary>
    public const string RecordsRule = "From what you dictate and record on this PC, never an import. The fastest counts dictations of 30 seconds or more.";

    /// <summary>The Records card before there is a best.</summary>
    public const string RecordsEmpty = "Your longest and fastest dictation, your best day and week, and your longest meeting and monologue appear here.";

    /// <summary>The Drop's note for a best just set: <c>Longest dictation yet</c> over <c>3 min 12 s · previous best 2 min 40 s</c>.</summary>
    public static DropLine BestNote(BestNews news, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(news);
        var title = news.Id switch
        {
            BestId.LongestDictation => "Longest dictation yet",
            BestId.FastestDictation => "Fastest dictation yet",
            BestId.MostWordsDay => "Most words in a day yet",
            BestId.BestWeek => "Most words in a week yet",
            BestId.LongestMeeting => "Longest meeting yet",
            _ => "Longest monologue yet",
        };
        return new DropLine(title, $"{BestValue(news.New, news.Unit, culture)} · previous best {BestValue(news.Old, news.Unit, culture)}", Yields: true);
    }

    // Week in review

    /// <summary>Which week the review is of: <c>Week of 28 Sep</c>.</summary>
    public static string ReviewWeek(WeekReview r, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(r);
        return $"Week of {ShortDate(r.Week, culture)}";
    }

    /// <summary>The review's numbers: words, time saved when there was some, meetings when there were some.</summary>
    public static IReadOnlyList<(string Value, string Label)> ReviewNumbers(WeekReview r, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(r);
        var numbers = new List<(string, string)> { (Count(r.Words, culture), r.Words == 1 ? "word" : "words") };
        if (r.SavedMs is > 0 and var saved)
        {
            numbers.Add((LibraryFormat.Duration(saved), "saved"));
        }
        if (r.Meetings > 0)
        {
            numbers.Add((LibraryFormat.Duration(r.MeetingMs), r.Meetings == 1 ? "in 1 meeting" : $"in {r.Meetings} meetings"));
        }
        return numbers;
    }

    /// <summary>The review's lines, gains and plain facts only: never a fall, never a red arrow.</summary>
    public static IReadOnlyList<string> ReviewLines(WeekReview r, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(r);
        culture ??= CultureInfo.CurrentCulture;
        var lines = new List<string>();
        if (r.BestDay is string best && r.BestDayWords is > 0 and var words && Day(best) is DateOnly day)
        {
            lines.Add($"Busiest day: {day.ToString("dddd", culture)}, {Count(words, culture)} words");
        }
        if (r.Wpm is > 0 and var wpm)
        {
            lines.Add(r.WpmGain is > 0 and var gain ? $"{wpm} wpm, {gain} faster than the four weeks before" : $"{wpm} wpm");
        }
        if (About(r.SavedAbout) is string about)
        {
            lines.Add($"Time saved: {about}");
        }
        if (r.PromisesKept is > 0 and var kept)
        {
            lines.Add(kept == 1 ? "1 promise from its meetings kept" : $"{kept} promises from its meetings kept");
        }
        return lines;
    }

    /// <summary><c>4 Oct</c> in the user's culture (YYYY-MM-DD as it came when unreadable).</summary>
    public static string ShortDate(string date, CultureInfo? culture = null) =>
        Day(date) is DateOnly day ? day.ToString("d MMM", culture ?? CultureInfo.CurrentCulture) : date;

    private static string Capitalised(string text) => text.Length == 0 ? text : char.ToUpperInvariant(text[0]) + text[1..];

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
