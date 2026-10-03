// The share card's numbers, as the Mac's ShareStat: which ones the user can tick, what each says,
// and their fixed order on the card. Numbers only, never a word from the library; the app draws
// the card (StatsShareCard) and puts it on the clipboard or in a file only when the user asks.
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
