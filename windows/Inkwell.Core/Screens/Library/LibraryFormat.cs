// How the library's times, lengths and records read on screen, as the Mac's LibraryFormat. Pure:
// the caller passes the moment and the calendar (time zone and culture), so the tests pin both.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>The time zone and culture the library's days and times are read in (the Mac's Calendar).</summary>
public sealed record LibraryCalendar(TimeZoneInfo Zone, CultureInfo Culture)
{
    /// <summary>This PC's zone and the user's culture, read when asked.</summary>
    public static LibraryCalendar Local => new(TimeZoneInfo.Local, CultureInfo.CurrentCulture);

    /// <summary><paramref name="moment"/> on this calendar's clock.</summary>
    public DateTime Clock(DateTimeOffset moment) => TimeZoneInfo.ConvertTime(moment, Zone).DateTime;

    /// <summary>Whether two moments fall on the same day here.</summary>
    public bool SameDay(DateTimeOffset a, DateTimeOffset b) => Clock(a).Date == Clock(b).Date;

    /// <summary>The start of the day <paramref name="now"/> is in.</summary>
    public DateTimeOffset StartOfDay(DateTimeOffset now) => Midnight(Clock(now).Date);

    /// <summary>The start of the week <paramref name="now"/> is in, the week starting on the culture's first day.</summary>
    public DateTimeOffset StartOfWeek(DateTimeOffset now)
    {
        var day = Clock(now).Date;
        var back = ((int)day.DayOfWeek - (int)Culture.DateTimeFormat.FirstDayOfWeek + 7) % 7;
        return Midnight(day.AddDays(-back));
    }

    /// <summary>Midnight of <paramref name="date"/> here; where a clock change skips midnight, the first moment the day has.</summary>
    private DateTimeOffset Midnight(DateTime date)
    {
        var local = DateTime.SpecifyKind(date, DateTimeKind.Unspecified);
        while (Zone.IsInvalidTime(local))
        {
            local = local.AddMinutes(15);
        }
        return new DateTimeOffset(local, Zone.GetUtcOffset(local));
    }
}

public static class LibraryFormat
{
    /// <summary>A position in a record: <c>12:41</c>, or <c>1:02:05</c> from an hour on.</summary>
    public static string Stamp(long ms)
    {
        var seconds = Math.Max(ms, 0) / 1000;
        var (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
        return h > 0
            ? string.Create(CultureInfo.InvariantCulture, $"{h}:{m:00}:{s:00}")
            : string.Create(CultureInfo.InvariantCulture, $"{m:00}:{s:00}");
    }

    /// <summary>A length: <c>under 1 min</c>, <c>42 min</c>, <c>2 h 10 min</c>.</summary>
    public static string Duration(long ms)
    {
        var minutes = (long)Math.Round(Math.Max(ms, 0) / 60_000.0, MidpointRounding.AwayFromZero);
        if (minutes < 1)
        {
            return "under 1 min";
        }
        if (minutes < 60)
        {
            return string.Create(CultureInfo.InvariantCulture, $"{minutes} min");
        }
        var (h, m) = (minutes / 60, minutes % 60);
        return m == 0
            ? string.Create(CultureInfo.InvariantCulture, $"{h} h")
            : string.Create(CultureInfo.InvariantCulture, $"{h} h {m} min");
    }

    /// <summary>The day relative to <paramref name="now"/>: <c>Today</c>, <c>Yesterday</c>, else <c>Mon 21 Sep</c>.</summary>
    public static string Day(DateTimeOffset date, DateTimeOffset now, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(calendar);
        var day = calendar.Clock(date).Date;
        var today = calendar.Clock(now).Date;
        if (day == today)
        {
            return "Today";
        }
        if (day == today.AddDays(-1))
        {
            return "Yesterday";
        }
        // The Mac asks for the locale's abbreviated weekday, day and month; .NET has no such
        // skeleton, so the order is fixed and the words are the culture's.
        return calendar.Clock(date).ToString("ddd d MMM", calendar.Culture);
    }

    /// <summary>The clock time: <c>14:02</c> (or the culture's form).</summary>
    public static string Time(DateTimeOffset date, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(calendar);
        return calendar.Clock(date).ToString("t", calendar.Culture);
    }

    /// <summary>Today's heading: <c>Saturday 27 September</c>.</summary>
    public static string LongDay(DateTimeOffset date, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(calendar);
        return calendar.Clock(date).ToString("dddd d MMMM", calendar.Culture);
    }

    /// <summary>A greeting for the hour.</summary>
    public static string Greeting(DateTimeOffset date, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(calendar);
        return calendar.Clock(date).Hour switch
        {
            >= 5 and < 12 => "Good morning",
            >= 12 and < 18 => "Good afternoon",
            _ => "Good evening",
        };
    }

    public static DateTimeOffset Date(long unixMs) => DateTimeOffset.FromUnixTimeMilliseconds(unixMs);

    /// <summary>A record's length, when it has ended.</summary>
    public static long? Length(RecordRow record)
    {
        ArgumentNullException.ThrowIfNull(record);
        return record.EndedAtUnixMs is long end ? Math.Max(end - record.StartedAtUnixMs, 0) : null;
    }

    /// <summary>
    /// What a record is called: its title, else the start of its words, else what it is. Never a
    /// placeholder like "Untitled" when there is something better to say.
    /// </summary>
    public static string Title(RecordRow record)
    {
        ArgumentNullException.ThrowIfNull(record);
        var title = record.Title?.Trim();
        if (!string.IsNullOrEmpty(title))
        {
            return title;
        }
        if (!string.IsNullOrEmpty(record.Preview))
        {
            return record.Preview;
        }
        return record.Kind switch
        {
            RecordKind.Meeting => "Meeting",
            RecordKind.Dictation => "Dictation",
            _ => "Imported file",
        };
    }

    /// <summary>A list row's second line: <c>Today · 14:02 · 42 min</c>.</summary>
    public static string ListLine(RecordRow record, DateTimeOffset now, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(record);
        var start = Date(record.StartedAtUnixMs);
        var parts = new List<string> { Day(start, now, calendar), Time(start, calendar) };
        if (Length(record) is long length)
        {
            parts.Add(Duration(length));
        }
        else if (record.Kind == RecordKind.Meeting)
        {
            parts.Add("recording");
        }
        return string.Join(" · ", parts);
    }

    /// <summary>A record's heading line: <c>Today 14:02 · 42 min · Zoom · You, Alex, Robin</c>.</summary>
    public static string HeaderLine(RecordRow record, IReadOnlyList<string> people, DateTimeOffset now, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(record);
        ArgumentNullException.ThrowIfNull(people);
        var start = Date(record.StartedAtUnixMs);
        var parts = new List<string> { $"{Day(start, now, calendar)} {Time(start, calendar)}" };
        if (Length(record) is long length)
        {
            parts.Add(Duration(length));
        }
        if (!string.IsNullOrWhiteSpace(record.SourceApp))
        {
            // The app as the user knows it ("WhatsApp"), never the core's identity for it
            // ("whatsapp.root.exe"): the well-known list, else the executable's stem.
            parts.Add(AppIdentity.Label(record.SourceApp, NoInstalledApps.Instance).Name);
        }
        if (people.Count > 0)
        {
            parts.Add(string.Join(", ", people));
        }
        return string.Join(" · ", parts);
    }

    /// <summary>A search match's third line: <c>Yesterday · 12:41</c> (the day of the record, where in it).</summary>
    public static string HitLine(SearchHit hit, DateTimeOffset now, LibraryCalendar calendar)
    {
        ArgumentNullException.ThrowIfNull(hit);
        return $"{Day(Date(hit.StartedAtUnixMs), now, calendar)} · {Stamp(hit.StartMs)}";
    }
}
