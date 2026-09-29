// Today's words, as the Mac's TodayScreen composes them: the date line and greeting, the counts at
// the foot, what is owed soon, and the last meeting's empty and failed states. Pure: the caller
// passes the moment, the time zone and the locale, so the tests pin all three. The inputs are
// plain values the Library and Owed screens' models hold (their counts, load states and items);
// Today reads them and asks for nothing itself.
//
// Times, positions, lengths and the greeting are LibraryFormat's (the Library screen's port). The
// date line is not: it follows the culture's day-and-month order ("Saturday 26 September" in
// en-GB, "Saturday September 26" in en-US) where LibraryFormat.LongDay fixes the order.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public static class TodayText
{
    /// <summary>How many promises Today lists.</summary>
    public const int OwedShown = 3;

    /// <summary>Today's heading: "Saturday 27 September" (the locale's day and month order).</summary>
    public static string LongDay(DateTimeOffset date, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(calendar);
        var local = calendar.Clock(date);
        return $"{local.ToString("dddd", calendar.Culture)} {local.ToString(calendar.Culture.DateTimeFormat.MonthDayPattern, calendar.Culture)}";
    }

    /// <summary>The counts at the foot: today's dictation and this week's meetings. A count that could not be read says so; it is never shown as zero. A count not answered yet is left out.</summary>
    /// <param name="today">library.stats since the start of today, when answered.</param>
    /// <param name="todayFailed">That question failed.</param>
    /// <param name="week">library.stats since the start of the week, when answered.</param>
    /// <param name="weekFailed">That question failed.</param>
    public static IReadOnlyList<string> StatsLines(
        LibraryStats? today, bool todayFailed, LibraryStats? week, bool weekFailed, CultureInfo culture)
    {
        ArgumentNullException.ThrowIfNull(culture);
        var lines = new List<string>();
        var dictated = today?.Kinds.FirstOrDefault(k => k.Kind == RecordKind.Dictation);
        if (dictated is not null)
        {
            lines.Add($"Dictated today · {dictated.Words.ToString("N0", culture)} words · {LibraryFormat.Duration(dictated.DurationMs)}");
        }
        else if (todayFailed)
        {
            lines.Add("Dictated today · couldn't be counted");
        }
        var met = week?.Kinds.FirstOrDefault(k => k.Kind == RecordKind.Meeting);
        if (met is not null)
        {
            lines.Add($"This week · {met.Records} {(met.Records == 1 ? "meeting" : "meetings")} · {LibraryFormat.Duration(met.DurationMs)}");
        }
        else if (weekFailed)
        {
            lines.Add("This week · couldn't be counted");
        }
        return lines;
    }

    /// <summary>The last meeting's place when there is none to show: "Couldn't load..." when the library could not be read, never "no meetings"; the empty note once loaded; null while asking or when there is one.</summary>
    public static string? LastMeetingNote(bool hasMeeting, bool loaded, bool failed) =>
        hasMeeting ? null
        : failed ? "Couldn't load your last meeting."
        : loaded ? "No meetings yet. When you join a call, Inkwell records both sides and the record lands here."
        : null;

    /// <summary>The last meeting's play button: "Play", or "Play from 12:41" from the first promise's moment.</summary>
    public static string PlayTitle(long fromMs) => fromMs > 0 ? $"Play from {LibraryFormat.Stamp(fromMs)}" : "Play";

    /// <summary>The first <see cref="OwedShown"/> of the Owed screen's list (soonest due first).</summary>
    public static IReadOnlyList<T> OwedSoon<T>(IReadOnlyList<T> items)
    {
        ArgumentNullException.ThrowIfNull(items);
        return items.Take(OwedShown).ToList();
    }

    /// <summary>The link to the Owed screen beside the heading: "All 7", or null with nothing owed.</summary>
    public static string? OwedAllTitle(int count) => count > 0 ? $"All {count}" : null;

    /// <summary>What Owed soon says instead of rows: the Owed model's failure when it has one, the empty note once loaded with nothing owed, else null.</summary>
    public static string? OwedNote(bool loaded, int count, string? failure) =>
        failure ?? (loaded && count == 0 ? "Nothing owed. Promises made in meetings show up here." : null);

    /// <summary>A promise's line: "Due tomorrow · Weekly sync"; <paramref name="due"/> is null for an undated one. Overdue ones are drawn in the alert colour (the Owed model's label says which).</summary>
    public static string OwedMeta(string? due, string? recordTitle) =>
        string.Join(" · ", new[] { due, recordTitle }.OfType<string>());

    /// <summary>A promise's play link: "▸ 12:41".</summary>
    public static string OwedPlayTitle(long saidAtMs) => $"▸ {LibraryFormat.Stamp(saidAtMs)}";

    /// <summary>The play link's accessible name.</summary>
    public static string OwedPlayLabel(long saidAtMs) => $"Play where it was said, {LibraryFormat.Stamp(saidAtMs)}";

    /// <summary>Owed soon's rows: the first <see cref="OwedShown"/> of the Owed model's list (soonest due first), as Today words them.</summary>
    public static IReadOnlyList<OwedSoonRow> OwedSoonRows(OwedModel owed, DateTimeOffset now)
    {
        ArgumentNullException.ThrowIfNull(owed);
        return OwedSoon(owed.Items).Select(item =>
        {
            var due = owed.Due(item, now);
            return new OwedSoonRow(
                item.Id, item.Text, OwedMeta(due is DueLabel.Undated ? null : due.Text, item.RecordTitle), due.IsOverdue,
                item.Record, item.SaidAtMs);
        }).ToList();
    }

    /// <summary>Where the last meeting's Play starts: the first promise's moment, else the start (the Mac's).</summary>
    public static long PlayFromMs(RecordDocument meeting)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        return meeting.Owed.Select(o => o.AtMs).OfType<long>().FirstOrDefault();
    }
}

/// <summary>One promise on Today: a circle marks it done, what is owed, when and where it was said.</summary>
/// <param name="Meta">"2 days overdue · Weekly sync".</param>
/// <param name="Overdue">Drawn in the alert colour.</param>
public sealed record OwedSoonRow(string Id, string Text, string Meta, bool Overdue, string Record, long? SaidAtMs)
{
    /// <summary>The circle's accessible name.</summary>
    public string DoneLabel => $"Mark done: {Text}";

    /// <summary>"▸ 12:41", or null when it was not placed in the record.</summary>
    public string? PlayTitle => SaidAtMs is long at ? TodayText.OwedPlayTitle(at) : null;

    public string? PlayLabel => SaidAtMs is long at ? TodayText.OwedPlayLabel(at) : null;
}
