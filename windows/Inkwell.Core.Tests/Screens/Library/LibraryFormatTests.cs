// How records read (the Mac's LibraryFormatTests), in a pinned zone and culture.
using System.Globalization;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.LibraryFixtures;

namespace Inkwell.Core.Tests.Screens;

public class LibraryFormatTests
{
    private static readonly LibraryCalendar Calendar =
        new(TimeZoneInfo.FindSystemTimeZoneById("Europe/Madrid"), CultureInfo.GetCultureInfo("en-GB"));

    /// <summary>Thursday 24 September 2026, 15:00 in Madrid.</summary>
    private static readonly DateTimeOffset Now = DateTimeOffset.FromUnixTimeSeconds(1_790_254_800);

    [Fact]
    public void StampsAndLengths()
    {
        Assert.Equal("12:41", LibraryFormat.Stamp(761_000));
        Assert.Equal("1:02:05", LibraryFormat.Stamp(3_725_000));
        Assert.Equal("00:00", LibraryFormat.Stamp(-5));
        Assert.Equal("under 1 min", LibraryFormat.Duration(20_000));
        Assert.Equal("42 min", LibraryFormat.Duration(42 * 60_000));
        Assert.Equal("2 h 10 min", LibraryFormat.Duration(130 * 60_000));
    }

    /// <summary>The Mac's testDaysReadAsToday_Yesterday_OrTheDate (underscores are refused here: CA1707).</summary>
    [Fact]
    public void DaysReadAsTodayYesterdayOrTheDate()
    {
        Assert.Equal("Today", LibraryFormat.Day(Now.AddSeconds(-3_600), Now, Calendar));
        Assert.Equal("Yesterday", LibraryFormat.Day(Now.AddSeconds(-86_400), Now, Calendar));
        // Older days read as the culture's short date; ICU versions spell the month differently
        // ("Sep", "Sept"), so only its shape is pinned.
        var older = LibraryFormat.Day(Now.AddSeconds(-3 * 86_400), Now, Calendar);
        Assert.StartsWith("Mon 21 Sep", older, StringComparison.Ordinal);
        Assert.Equal("15:00", LibraryFormat.Time(Now, Calendar));
        Assert.Equal("Good afternoon", LibraryFormat.Greeting(Now, Calendar));
    }

    [Fact]
    public void ARecordIsCalledByItsTitleThenItsWordsNeverUntitled()
    {
        var titled = Rows($$"""{"type":"library.records","more":false,"records":[{{Row("a", start: 0, title: "Launch moves to the 14th")}}]}""")[0];
        Assert.Equal("Launch moves to the 14th", LibraryFormat.Title(titled));
        var dictated = Rows("""
            {"type":"library.records","more":false,"records":[{"record":"d","kind":"dictation","started_at_unix_ms":0,"revision":1,"has_audio":false,"preview":"Remind me to book the room"}]}
            """)[0];
        Assert.Equal("Remind me to book the room", LibraryFormat.Title(dictated));
        var bare = Rows($$"""{"type":"library.records","more":false,"records":[{{Row("m", start: 0, title: "  ")}}]}""")[0];
        Assert.Equal("Meeting", LibraryFormat.Title(bare));
    }

    [Fact]
    public void ListAndHeaderLines()
    {
        var start = Now.ToUnixTimeMilliseconds() - 60 * 60_000;
        var meeting = Rows($$"""
            {"type":"library.records","more":false,"records":[{"record":"m","kind":"meeting","started_at_unix_ms":{{start}},"ended_at_unix_ms":{{start + 42 * 60_000}},"source_app":"Zoom","revision":2,"has_audio":true}]}
            """)[0];
        Assert.Equal("Today · 14:00 · 42 min", LibraryFormat.ListLine(meeting, Now, Calendar));
        Assert.Equal("Today 14:00 · 42 min · Zoom · You, Alex, Robin", LibraryFormat.HeaderLine(meeting, ["You", "Alex", "Robin"], Now, Calendar));
    }

    /// <summary>Windows: a record's app is named as the user knows it, never by the core's identity (its executable).</summary>
    [Fact]
    public void AHeaderLineNamesTheAppNotItsExecutable()
    {
        var start = Now.ToUnixTimeMilliseconds() - 60 * 60_000;
        string Header(string app) => LibraryFormat.HeaderLine(Rows($$"""
            {"type":"library.records","more":false,"records":[{"record":"m","kind":"meeting","started_at_unix_ms":{{start}},"ended_at_unix_ms":{{start + 6 * 60_000}},"source_app":"{{app}}","revision":2,"has_audio":true}]}
            """)[0], ["Them"], Now, Calendar);
        Assert.Equal("Today 14:00 · 6 min · WhatsApp · Them", Header("whatsapp.root.exe"));
        Assert.Equal("Today 14:00 · 6 min · Microsoft Teams · Them", Header("ms-teams.exe"));
        Assert.Equal("Today 14:00 · 6 min · Callapp · Them", Header("callapp.exe")); // not well known: its stem
    }

    /// <summary>Windows addition: Today's counts start at midnight and at the culture's first day of the week, in the calendar's zone (the Mac leaves this to Foundation's Calendar).</summary>
    [Fact]
    public void TodayAndThisWeekStartAtMidnightOnTheCulturesFirstDay()
    {
        // Thursday 24 September 2026 in Madrid (UTC+2): the day from 00:00 local, the week from Monday 21.
        Assert.Equal(DateTimeOffset.Parse("2026-09-24T00:00:00+02:00", CultureInfo.InvariantCulture), Calendar.StartOfDay(Now));
        Assert.Equal(DateTimeOffset.Parse("2026-09-21T00:00:00+02:00", CultureInfo.InvariantCulture), Calendar.StartOfWeek(Now));
        var sundayFirst = Calendar with { Culture = CultureInfo.GetCultureInfo("en-US") };
        Assert.Equal(DateTimeOffset.Parse("2026-09-20T00:00:00+02:00", CultureInfo.InvariantCulture), sundayFirst.StartOfWeek(Now));
    }
}
