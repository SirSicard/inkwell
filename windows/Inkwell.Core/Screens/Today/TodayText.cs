// Today's words, as the Mac's TodayScreen composes them: the date line and greeting, the counts at
// the foot, what is owed soon, and the last meeting's empty and failed states. Pure: the caller
// passes the moment, the time zone and the locale, so the tests pin all three. The inputs are
// plain values the Library and Owed screens' models hold (their counts, load states and items);
// Today reads them and asks for nothing itself.
//
// Time, Stamp and Duration are the Mac's LibraryFormat.time/stamp/duration, which the Library
// screen's port owns; they are repeated here only so Today builds on its own.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public static class TodayText
{
    /// <summary>How many promises Today lists.</summary>
    public const int OwedShown = 3;

    /// <summary>Today's heading: "Saturday 27 September" (the locale's day and month order).</summary>
    public static string LongDay(DateTimeOffset date, TimeZoneInfo zone, CultureInfo culture)
    {
        ArgumentNullException.ThrowIfNull(culture);
        var local = TimeZoneInfo.ConvertTime(date, zone);
        return $"{local.ToString("dddd", culture)} {local.ToString(culture.DateTimeFormat.MonthDayPattern, culture)}";
    }

    /// <summary>A greeting for the hour where the user is.</summary>
    public static string Greeting(DateTimeOffset date, TimeZoneInfo zone) =>
        TimeZoneInfo.ConvertTime(date, zone).Hour switch
        {
            >= 5 and < 12 => "Good morning",
            >= 12 and < 18 => "Good afternoon",
            _ => "Good evening",
        };

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
            lines.Add($"Dictated today · {dictated.Words.ToString("N0", culture)} words · {Duration(dictated.DurationMs)}");
        }
        else if (todayFailed)
        {
            lines.Add("Dictated today · couldn't be counted");
        }
        var met = week?.Kinds.FirstOrDefault(k => k.Kind == RecordKind.Meeting);
        if (met is not null)
        {
            lines.Add($"This week · {met.Records} {(met.Records == 1 ? "meeting" : "meetings")} · {Duration(met.DurationMs)}");
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
    public static string PlayTitle(long fromMs) => fromMs > 0 ? $"Play from {Stamp(fromMs)}" : "Play";

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
    public static string OwedPlayTitle(long saidAtMs) => $"▸ {Stamp(saidAtMs)}";

    /// <summary>The play link's accessible name.</summary>
    public static string OwedPlayLabel(long saidAtMs) => $"Play where it was said, {Stamp(saidAtMs)}";

    /// <summary>The clock time: "14:02" (or the locale's form).</summary>
    public static string Time(DateTimeOffset date, TimeZoneInfo zone, CultureInfo culture)
    {
        ArgumentNullException.ThrowIfNull(culture);
        return TimeZoneInfo.ConvertTime(date, zone).ToString(culture.DateTimeFormat.ShortTimePattern, culture);
    }

    /// <summary>A position in a record: "12:41", or "1:02:05" from an hour on.</summary>
    public static string Stamp(long ms)
    {
        var seconds = Math.Max(ms, 0) / 1000;
        var (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
        return h > 0
            ? string.Create(CultureInfo.InvariantCulture, $"{h}:{m:00}:{s:00}")
            : string.Create(CultureInfo.InvariantCulture, $"{m:00}:{s:00}");
    }

    /// <summary>A length: "under 1 min", "42 min", "2 h 10 min".</summary>
    public static string Duration(long ms)
    {
        var minutes = (long)Math.Round(Math.Max(ms, 0) / 60_000.0, MidpointRounding.AwayFromZero);
        if (minutes < 1)
        {
            return "under 1 min";
        }
        if (minutes < 60)
        {
            return $"{minutes} min";
        }
        var (h, m) = (minutes / 60, minutes % 60);
        return m == 0 ? $"{h} h" : $"{h} h {m} min";
    }
}
