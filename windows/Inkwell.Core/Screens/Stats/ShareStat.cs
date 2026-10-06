// The share card's numbers, as the Mac's ShareStat: which ones the user can tick, what each says,
// and their fixed order on the card. Numbers only, never a word from the library; the app draws
// the card (StatsShareCard) and puts it on the clipboard or in a file only when the user asks.
// Beside the numbers it can carry a records line, a seal for each milestone reached that the user
// keeps ticked, and the heatmap: that last one only once the user turns it on
// (stats.share_heatmap, off unless set), since a grid of days shows which days they worked.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>A number the card can carry, in the card's order.</summary>
public enum ShareStat
{
    WordsAll,
    WordsWeek,
    Speed,
    Saved,
    Streak,
    MeetingHours,
    TalkTime,
    PromisesKept,
}

/// <summary>One line of the card: a number and what it counts.</summary>
public sealed record ShareLine(string Value, string Label);

public static class ShareStats
{
    public static IReadOnlyList<ShareStat> All { get; } = Enum.GetValues<ShareStat>();

    /// <summary>What the card carries at first.</summary>
    public static IReadOnlySet<ShareStat> Defaults { get; } = new HashSet<ShareStat> { ShareStat.WordsAll, ShareStat.Saved, ShareStat.Streak };

    /// <summary>The tick's name.</summary>
    public static string Title(this ShareStat stat) => stat switch
    {
        ShareStat.WordsAll => "Words dictated",
        ShareStat.WordsWeek => "Words this week",
        ShareStat.Speed => "Speaking speed this week",
        ShareStat.Saved => "Time saved",
        ShareStat.Streak => "Streak",
        ShareStat.MeetingHours => "Hours in meetings this month",
        ShareStat.TalkTime => "Talk time this month",
        ShareStat.PromisesKept => "Promises kept this month",
        _ => throw new ArgumentOutOfRangeException(nameof(stat)),
    };

    /// <summary>The tick's name, saying when the library has no such number yet.</summary>
    public static string TickTitle(this ShareStat stat, StatsCounted c) =>
        stat.Available(c) ? stat.Title() : $"{stat.Title()} (none yet)";

    /// <summary>Its line on the card, or null while the library has no such number yet.</summary>
    public static ShareLine? Line(this ShareStat stat, StatsCounted c, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(c);
        var d = c.Dictation;
        switch (stat)
        {
            case ShareStat.WordsAll:
                return d.DictationsAll > 0 ? new ShareLine(StatsFormat.Count(d.WordsAll, culture), "words dictated") : null;
            case ShareStat.WordsWeek:
                return d.DictationsAll > 0 ? new ShareLine(StatsFormat.Count(d.WordsWeek, culture), "words this week") : null;
            case ShareStat.Speed:
                return d.WpmWeek is long wpm ? new ShareLine($"{wpm} wpm", "speaking speed this week") : null;
            case ShareStat.Saved:
                return d.SavedMsAll > 0 ? new ShareLine(LibraryFormat.Duration(d.SavedMsAll), $"saved vs typing at {c.TypingWpm} wpm") : null;
            case ShareStat.Streak:
                // A hidden streak is offered nowhere.
                if (d.StreakHidden == true)
                {
                    return null;
                }
                if (d.StreakDays >= 2)
                {
                    return new ShareLine($"{d.StreakDays} active days", "dictation streak");
                }
                return d.LongestStreakDays >= 2 ? new ShareLine($"{d.LongestStreakDays} active days", "longest dictation streak") : null;
            case ShareStat.MeetingHours:
                return c.MeetingsMonth.Meetings > 0 ? new ShareLine(LibraryFormat.Duration(c.MeetingsMonth.RecordedMs), "in meetings this month") : null;
            case ShareStat.TalkTime:
                return StatsFormat.TalkShare(c.MeetingsMonth.YouMs, c.MeetingsMonth.ThemMs) is (int you, int them)
                    ? new ShareLine($"{you} % / {them} %", "talk time this month, you / them")
                    : null;
            case ShareStat.PromisesKept:
                var p = c.PromisesMonth;
                return p.Made > 0 ? new ShareLine($"{p.Kept} of {p.Made}", "promises kept this month") : null;
            default:
                throw new ArgumentOutOfRangeException(nameof(stat));
        }
    }

    /// <summary>Whether the library has this number yet.</summary>
    public static bool Available(this ShareStat stat, StatsCounted c) => stat.Line(c) is not null;

    /// <summary>The card's lines: the ticked ones the library has, in the card's own order.</summary>
    public static IReadOnlyList<ShareLine> Lines(StatsCounted c, IReadOnlySet<ShareStat> selected, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(selected);
        return All.Where(selected.Contains).Select(s => s.Line(c, culture)).OfType<ShareLine>().ToList();
    }

    /// <summary>What Narrator hears for the card's preview.</summary>
    public static string Spoken(IReadOnlyList<ShareLine> lines)
    {
        ArgumentNullException.ThrowIfNull(lines);
        return lines.Count == 0 ? "The card is empty" : "The card: " + string.Join(", ", lines.Select(l => $"{l.Value} {l.Label}"));
    }

    /// <summary>The card's foot.</summary>
    public const string Foot = "Counted on my PC";

    /// <summary>The file name Save offers.</summary>
    public const string FileName = "Inkwell stats.png";
}

/// <summary>A seal the card can carry: a milestone reached, by its name.</summary>
/// <param name="Id">The milestone's id.</param>
/// <param name="Mark">Its count, short: <c>10k</c>, <c>30</c>.</param>
/// <param name="Name">Its name: <c>A notebook</c>.</param>
public sealed record ShareSeal(string Id, string Mark, string Name)
{
    /// <summary>The seals the library has: each milestone reached, in the core's order. A hidden streak's are not listed by the core.</summary>
    public static IReadOnlyList<ShareSeal> Available(StatsCounted c)
    {
        ArgumentNullException.ThrowIfNull(c);
        return c.Milestones.Where(m => m.Reached && m.Name is not null)
            .Select(m => new ShareSeal(m.Id, StatsFormat.SealMark(m.Kind, m.Threshold), StatsFormat.MilestoneNamed(m.Name!.Value)))
            .ToList();
    }
}

/// <summary>The card's records line: up to three bests, in the core's order.</summary>
public static class ShareRecords
{
    public const int Most = 3;

    /// <summary>
    /// <c>Longest dictation: 3 min 12 s · Fastest dictation: 168 wpm · Most words in a day: 2,340</c>;
    /// null without a best. Each record is kept whole: the line breaks only between them.
    /// </summary>
    public static string? Line(StatsCounted c, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(c);
        var parts = (c.Bests ?? []).Take(Most).Select(b =>
        {
            var name = StatsFormat.BestName(b.Id);
            var value = b.Unit == BestUnit.Words ? StatsFormat.Count(b.Value, culture) : StatsFormat.BestValue(b.Value, b.Unit, culture);
            return $"{char.ToUpperInvariant(name[0])}{name[1..]}: {value}".Replace(' ', '\u00A0');
        }).ToList();
        return parts.Count == 0 ? null : string.Join(" · ", parts);
    }
}

/// <summary>Everything on the card, in its order: the numbers, the records line, the heatmap, the seals.</summary>
/// <param name="Heatmap">The heatmap's days, shaded 0 to 4, a column per week; null when not on the card.</param>
public sealed record ShareContent(IReadOnlyList<ShareLine> Lines, string? Records, IReadOnlyList<int>? Heatmap, IReadOnlyList<ShareSeal> Seals)
{
    public static ShareContent Empty { get; } = new([], null, null, []);

    public bool IsEmpty => Lines.Count == 0 && Records is null && Heatmap is null && Seals.Count == 0;

    /// <summary>
    /// What the card's choices put on it. The heatmap only with <paramref name="heatmap"/> (the
    /// user's stats.share_heatmap) and a dictation to show.
    /// </summary>
    public static ShareContent Make(StatsCounted c, IReadOnlySet<ShareStat> selected, bool records, bool heatmap, IReadOnlySet<string> seals, CultureInfo? culture = null)
    {
        ArgumentNullException.ThrowIfNull(c);
        ArgumentNullException.ThrowIfNull(seals);
        var cells = StatsFormat.Heatmap(c.Dictation);
        return new ShareContent(
            ShareStats.Lines(c, selected, culture),
            records ? ShareRecords.Line(c, culture) : null,
            heatmap && c.Dictation.DictationsAll > 0 && cells.Count > 0 ? cells.Select(cell => cell.Level).ToList() : null,
            ShareSeal.Available(c).Where(s => seals.Contains(s.Id)).ToList());
    }

    /// <summary>What Narrator hears for the preview.</summary>
    public string Spoken()
    {
        if (IsEmpty)
        {
            return "The card is empty";
        }
        var parts = Lines.Select(l => $"{l.Value} {l.Label}").ToList();
        if (Records is not null)
        {
            parts.Add(Records.Replace('\u00A0', ' '));
        }
        if (Heatmap is not null)
        {
            parts.Add("the heatmap of the last 12 weeks");
        }
        if (Seals.Count > 0)
        {
            parts.Add("seals: " + string.Join(", ", Seals.Select(s => s.Name)));
        }
        return "The card: " + string.Join(", ", parts);
    }
}
