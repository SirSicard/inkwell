// Today's words: the date line and greeting, the counts at the foot (a count that could not be
// read says so, never zero), what is owed soon, and the ink zone's foot (as the Mac's
// DetectionStateTests, and the key the core bound in Windows' words). The Mac tests these through
// its views and LibraryFormat; here they are the TodayText and RecordControlsModel functions.
using System.Globalization;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class TodayTextTests
{
    private static readonly CultureInfo EnGb = CultureInfo.GetCultureInfo("en-GB");

    [Fact]
    public void TheDateLineAndGreetingFollowTheUsersZone()
    {
        var saturday = new DateTimeOffset(2026, 9, 26, 15, 0, 0, TimeSpan.Zero);
        Assert.Equal("Saturday 26 September", TodayText.LongDay(saturday, TimeZoneInfo.Utc, EnGb));
        Assert.Equal("Good afternoon", TodayText.Greeting(saturday, TimeZoneInfo.Utc));
        Assert.Equal("Good morning", TodayText.Greeting(saturday.AddHours(-10), TimeZoneInfo.Utc));
        Assert.Equal("Good evening", TodayText.Greeting(saturday.AddHours(4), TimeZoneInfo.Utc));
        Assert.Equal("Good evening", TodayText.Greeting(saturday.AddHours(-11), TimeZoneInfo.Utc)); // 04:00
        var plusTen = TimeZoneInfo.CreateCustomTimeZone("plus-ten", TimeSpan.FromHours(10), "plus-ten", "plus-ten");
        Assert.Equal("Sunday 27 September", TodayText.LongDay(saturday, plusTen, EnGb));
        Assert.Equal("15:00", TodayText.Time(saturday, TimeZoneInfo.Utc, EnGb));
    }

    [Fact]
    public void TheCountsSayWhenTheyCouldNotBeReadAndNeverShowZero()
    {
        var today = Ev.Of<LibraryStats>("""{"type":"library.stats","since_unix_ms":0,"far_silent_meetings":0,"kinds":[{"kind":"dictation","records":9,"words":1234,"duration_ms":2520000}]}""");
        var week = Ev.Of<LibraryStats>("""{"type":"library.stats","since_unix_ms":0,"far_silent_meetings":0,"kinds":[{"kind":"meeting","records":1,"words":900,"duration_ms":7800000}]}""");
        Assert.Equal(
            ["Dictated today · 1,234 words · 42 min", "This week · 1 meeting · 2 h 10 min"],
            TodayText.StatsLines(today, false, week, false, EnGb));
        Assert.Equal(
            ["Dictated today · couldn't be counted", "This week · couldn't be counted"],
            TodayText.StatsLines(null, true, null, true, EnGb));
        Assert.Empty(TodayText.StatsLines(null, false, null, false, EnGb));
    }

    [Fact]
    public void TheLastMeetingSaysWhenItCouldNotBeLoaded()
    {
        Assert.Equal("Couldn't load your last meeting.", TodayText.LastMeetingNote(hasMeeting: false, loaded: false, failed: true));
        Assert.StartsWith("No meetings yet.", TodayText.LastMeetingNote(hasMeeting: false, loaded: true, failed: false), StringComparison.Ordinal);
        Assert.Null(TodayText.LastMeetingNote(hasMeeting: false, loaded: false, failed: false));
        Assert.Null(TodayText.LastMeetingNote(hasMeeting: true, loaded: true, failed: false));
        Assert.Equal("Play", TodayText.PlayTitle(0));
        Assert.Equal("Play from 12:41", TodayText.PlayTitle(761_000));
        Assert.Equal("Play from 1:02:05", TodayText.PlayTitle(3_725_000));
    }

    [Fact]
    public void OwedSoonListsTheFirstThree()
    {
        Assert.Equal(["a", "b", "c"], TodayText.OwedSoon(["a", "b", "c", "d"]));
        Assert.Equal("All 4", TodayText.OwedAllTitle(4));
        Assert.Null(TodayText.OwedAllTitle(0));
        Assert.Equal("Nothing owed. Promises made in meetings show up here.", TodayText.OwedNote(loaded: true, count: 0, failure: null));
        Assert.Null(TodayText.OwedNote(loaded: false, count: 0, failure: null));
        Assert.Equal("Couldn't mark it done: x", TodayText.OwedNote(loaded: true, count: 2, failure: "Couldn't mark it done: x"));
        Assert.Equal("Due tomorrow · Weekly sync", TodayText.OwedMeta("Due tomorrow", "Weekly sync"));
        Assert.Equal("Weekly sync", TodayText.OwedMeta(null, "Weekly sync"));
        Assert.Equal("▸ 00:42", TodayText.OwedPlayTitle(42_000));
        Assert.Equal("Play where it was said, 00:42", TodayText.OwedPlayLabel(42_000));
    }

    /// From the Mac's DetectionStateTests: Today follows the core's detection state, not the
    /// setting.
    [Fact]
    public void TodayFollowsTheCoresDetectionStateNotTheSetting()
    {
        var store = new CoreStore();
        Assert.Equal(" ", RecordControlsModel.ListeningText(recording: false, listening: store.Listening));
        store.Apply([Ev.Of("""{"type":"meeting.detection","listening":false,"message":"couldn't read the detection setting: database is locked"}""")]);
        Assert.False(store.Listening);
        Assert.Equal("Not listening for meetings", RecordControlsModel.ListeningText(false, store.Listening));
        Assert.IsType<NoticeKind.DetectionUnavailable>(store.Notices[^1].Kind);
        store.Apply([Ev.Of("""{"type":"meeting.detection","listening":true}""")]);
        Assert.Equal("Listening for meetings", RecordControlsModel.ListeningText(false, store.Listening));
        Assert.Equal("Recording", RecordControlsModel.ListeningText(true, true));
    }

    /// Windows only: the Mac's fixed "Hold fn to dictate" names the key the core bound.
    [Fact]
    public void TheDictationLineNamesTheKeyTheCoreBound()
    {
        var controls = new RecordControlsModel();
        Assert.Null(controls.DictateText);
        controls.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control"}"""));
        Assert.Equal("Hold Right Ctrl to dictate", controls.DictateText);
        var changes = controls.Changes;
        controls.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control"}"""));
        Assert.Equal(changes, controls.Changes);
        controls.Apply(Ev.Of("""{"type":"dictation.ready","key":"ctrl+shift+space"}"""));
        Assert.Equal("Hold Ctrl+Shift+Space to dictate", controls.DictateText);
        Assert.Equal("Right Windows key", KeyNames.Display("right_win"));
        Assert.Equal("Right Alt", KeyNames.Display("right_alt"));
        Assert.Equal("Right Shift", KeyNames.Display("right_shift"));
        Assert.Equal("F13", KeyNames.Display("f13"));
        Assert.Equal("Alt+D", KeyNames.Display("alt+d"));
        Assert.DoesNotContain("Mac", RecordControlsModel.RecordNowHint, StringComparison.Ordinal);
    }
}
