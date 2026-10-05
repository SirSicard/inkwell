// The Stats screen's model and words, as the Mac's StatsModelTests and StatsFormatTests: what it
// asks the core (stats.get on the user's calendar, milestones.check when a dictation or a meeting
// ends), what it keeps of the answers, and how the numbers read. The counting is the core's
// (ink-ffi's stats tests); nothing here counts.
using System.Globalization;
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class StatsModelTests
{
    private static readonly CultureInfo Gb = CultureInfo.GetCultureInfo("en-GB");

    /// <summary>3 October 2026, noon UTC (the Mac's tests' clock).</summary>
    private static readonly DateTimeOffset Now = DateTimeOffset.FromUnixTimeSeconds(1_791_028_800);

    private sealed class FakeWakes : IWakeScheduler
    {
        public List<(TimeSpan Delay, Action Wake, Handle Handle)> Scheduled { get; } = [];

        public IDisposable After(TimeSpan delay, Action wake)
        {
            var handle = new Handle();
            Scheduled.Add((delay, wake, handle));
            return handle;
        }

        public sealed class Handle : IDisposable
        {
            public bool Disposed { get; private set; }

            public void Dispose() => Disposed = true;
        }
    }

    private sealed class Rig
    {
        public Sent Sent { get; } = new();
        public FakeWakes Wakes { get; } = new();
        public StatsModel Stats { get; }

        public TimeZoneInfo Zone { get; set; } = TimeZoneInfo.Utc;

        public Rig() => Stats = new StatsModel(Sent.Send, Wakes, () => Zone, Gb, () => Now);

        public List<string> Names() => Sent.Commands.Select(c => c.Name).ToList();

        public string LastRef(string cmd) => Sent.Commands.Last(c => c.Name == cmd).CommandId!;

        public JsonElement LastFields(string cmd) => JsonDocument.Parse(Sent.Commands.Last(c => c.Name == cmd).Json).RootElement;
    }

    /// <summary>A stats.counted answer, with the dictation part given and the rest empty unless given (the Mac's statsCounted).</summary>
    internal static string Counted(
        string reference, (long Today, long Week, long All) words = default, long dictations = 0, long? wpmWeek = null, long? wpmAverage = null,
        long savedWeek = 0, long savedAll = 0, long streak = 0, long longest = 0, long[]? heatmap = null,
        string meetings = """{"meetings":0,"recorded_ms":0,"you_ms":0,"them_ms":0,"longest_monologue_ms":0,"questions":0}""",
        string promises = """{"made":0,"kept":0,"open":0,"overdue":0}""",
        IReadOnlySet<string>? reached = null, long typingWpm = 40)
    {
        heatmap ??= new long[83];
        reached ??= new HashSet<string>();
        var dictation = $"\"words_today\":{words.Today},\"words_week\":{words.Week},\"words_all\":{words.All},\"dictations_all\":{dictations},"
            + $"\"saved_ms_week\":{savedWeek},\"saved_ms_all\":{savedAll},\"streak_days\":{streak},\"longest_streak_days\":{longest},"
            + $"\"heatmap_first_day\":\"2026-07-13\",\"heatmap_words\":[{string.Join(",", heatmap)}]";
        if (wpmWeek is long w)
        {
            dictation += $",\"wpm_week\":{w}";
        }
        if (wpmAverage is long a)
        {
            dictation += $",\"wpm_average\":{a}";
        }
        (string Id, string Kind, int Threshold)[] all =
        [
            ("words_1000", "words", 1000), ("words_10000", "words", 10000), ("words_50000", "words", 50000), ("words_100000", "words", 100000),
            ("streak_7", "streak", 7), ("streak_30", "streak", 30), ("streak_100", "streak", 100),
        ];
        var milestones = string.Join(",", all.Select(m =>
            $"{{\"id\":\"{m.Id}\",\"kind\":\"{m.Kind}\",\"threshold\":{m.Threshold},\"reached\":{(reached.Contains(m.Id) ? "true" : "false")}}}"));
        return $"{{\"type\":\"stats.counted\",\"ref\":\"{reference}\",\"today\":\"2026-10-03\",\"typing_wpm\":{typingWpm},\"dictation\":{{{dictation}}},"
            + $"\"meetings_month\":{meetings},\"meetings_all\":{meetings},\"promises_month\":{promises},\"promises_all\":{promises},\"milestones\":[{milestones}]}}";
    }

    [Fact]
    public void LoadAsksOnTheUsersCalendarAndKeepsOnlyTheNewestAnswer()
    {
        var rig = new Rig();
        rig.Stats.Reload();
        var first = rig.LastRef("stats.get");
        var asked = rig.LastFields("stats.get");
        Assert.Equal(1, asked.GetProperty("week_start").GetInt32()); // en-GB weeks start on Monday
        var offsets = asked.GetProperty("utc_offsets");
        Assert.Equal(1, offsets.GetArrayLength()); // UTC never changes
        Assert.Equal(0, offsets[0].GetProperty("minutes").GetInt32());
        Assert.Equal(StatsModel.Load.Loading, rig.Stats.LoadState);

        rig.Stats.Reload();
        var second = rig.LastRef("stats.get");
        Assert.NotEqual(first, second);
        rig.Stats.Apply(Ev.Of(Counted(first, words: (1, 1, 1))));
        Assert.Null(rig.Stats.Counted); // a stale answer is not shown
        rig.Stats.Apply(Ev.Of(Counted(second, words: (5, 50, 500))));
        Assert.Equal(500, rig.Stats.Counted?.Dictation.WordsAll);
        Assert.Equal(StatsModel.Load.Loaded, rig.Stats.LoadState);
    }

    /// <summary>Each question reads the zone afresh: a PC that moved zones while Inkwell ran counts in the new one.</summary>
    [Fact]
    public void EachQuestionTakesTheZoneAsItIsNow()
    {
        var rig = new Rig();
        rig.Stats.Reload();
        Assert.Equal(0, rig.LastFields("stats.get").GetProperty("utc_offsets")[0].GetProperty("minutes").GetInt32());
        rig.Zone = TimeZoneInfo.FindSystemTimeZoneById("Asia/Tokyo");
        rig.Stats.CheckMilestones();
        Assert.Equal(540, rig.LastFields("milestones.check").GetProperty("utc_offsets")[0].GetProperty("minutes").GetInt32());
    }

    [Fact]
    public void AFailureIsSaidNeverShownAsZero()
    {
        var rig = new Rig();
        rig.Stats.Reload();
        var failed = Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"stats.get","id":"{{rig.LastRef("stats.get")}}","message":"the library: disk I/O error"}""");
        Assert.True(StatsModel.Handles(failed));
        Assert.False(StatsModel.Handles(failed with { Command = "milestones.check", Id = "milestones-1" })); // logged, not shown
        rig.Stats.Apply(failed);
        Assert.Equal(StatsModel.Load.Failed, rig.Stats.LoadState);
        Assert.Null(rig.Stats.Counted);
    }

    /// <summary>
    /// A zone with daylight saving sends each change, oldest first, so an old record keeps the
    /// offset it was made under; weeks start where the user's culture starts them.
    /// </summary>
    [Fact]
    public void TheCalendarFieldsCarryEveryOffsetChange()
    {
        var madrid = StatsModel.UtcOffsets(TimeZoneInfo.FindSystemTimeZoneById("Europe/Madrid"), Now);
        Assert.True(madrid.Count > 15, "two changes a year for ten years");
        Assert.Equal(madrid.Select(o => o.FromUnixMs).Order(), madrid.Select(o => o.FromUnixMs)); // oldest first
        Assert.Equal([60, 120], madrid.Select(o => o.Minutes).Distinct().Order());
        Assert.Equal(120, madrid[^1].Minutes); // summer time on 3 October
        Assert.True(madrid.Count < 400);
        Assert.Equal([540], StatsModel.UtcOffsets(TimeZoneInfo.FindSystemTimeZoneById("Asia/Tokyo"), Now).Select(o => o.Minutes));

        // Not only daylight saving: Pyongyang moved its standard time from UTC+8:30 to UTC+9 on
        // 2018-05-04 at 23:30 local (15:00 UTC).
        var pyongyang = StatsModel.UtcOffsets(TimeZoneInfo.FindSystemTimeZoneById("Asia/Pyongyang"), Now);
        Assert.Equal([510, 540], pyongyang.Select(o => o.Minutes));
        Assert.Equal(1_525_446_000_000, pyongyang[^1].FromUnixMs);
        // A daylight-saving change lands on its second too: Madrid's on 2026-03-29 01:00 UTC.
        Assert.Contains(madrid, o => o.FromUnixMs == 1_774_746_000_000 && o.Minutes == 120);

        Assert.Equal(7, StatsModel.IsoWeekStart(CultureInfo.GetCultureInfo("en-US"))); // Sunday
        Assert.Equal(1, StatsModel.IsoWeekStart(Gb)); // Monday
        var saturday = (CultureInfo)Gb.Clone();
        saturday.DateTimeFormat.FirstDayOfWeek = DayOfWeek.Saturday;
        Assert.Equal(6, StatsModel.IsoWeekStart(saturday));
    }

    /// <summary>
    /// Milestones are checked when the core is ready (the core notes what is already reached) and
    /// whenever a dictation is saved or a meeting ends; a dictation that was not saved changes nothing.
    /// </summary>
    [Fact]
    public void MilestonesAreCheckedAtLaunchAndWhenSomethingIsSaved()
    {
        var rig = new Rig();
        rig.Stats.Apply(Ev.Of("""{"type":"core.ready","version":"1.0.0","abi":2}"""));
        Assert.Contains("milestones.check", rig.Names());
        Assert.Equal(
            ["stats.celebrate", "stats.typing_wpm"],
            rig.Sent.Commands.OfType<CoreCommand.SettingGet>().Select(g => g.Key.Key()).Order());

        rig.Sent.Commands.Clear();
        rig.Stats.Apply(Ev.Of("""{"type":"dictation.inserted","outcome":"pasted","text":"x"}"""));
        Assert.Empty(rig.Sent.Commands); // not saved: nothing new to count
        rig.Stats.Apply(Ev.Of("""{"type":"dictation.inserted","outcome":"pasted","record":"r1","text":"x"}"""));
        Assert.Equal(["milestones.check"], rig.Names());
        rig.Sent.Commands.Clear();
        rig.Stats.Apply(Ev.Of("""{"type":"meeting.finished","record":"m1"}"""));
        Assert.Equal(["milestones.check"], rig.Names());
    }

    /// <summary>While the screen shows, a new dictation counts at once.</summary>
    [Fact]
    public void AShownScreenCountsAgainWhenSomethingIsSaved()
    {
        var rig = new Rig();
        rig.Stats.ScreenAppeared();
        Assert.Equal(["stats.get"], rig.Names());
        rig.Sent.Commands.Clear();
        rig.Stats.Apply(Ev.Of("""{"type":"dictation.inserted","outcome":"pasted","record":"r1","text":"x"}"""));
        Assert.Equal(["milestones.check", "stats.get"], rig.Names().Order());
        rig.Stats.ScreenDisappeared();
        rig.Sent.Commands.Clear();
        rig.Stats.Apply(Ev.Of("""{"type":"dictation.inserted","outcome":"pasted","record":"r2","text":"x"}"""));
        Assert.Equal(["milestones.check"], rig.Names());
    }

    [Fact]
    public void AMilestoneReachedIsCelebratedWithOneLine()
    {
        var rig = new Rig();
        rig.Stats.CheckMilestones();
        var reference = rig.LastRef("milestones.check");
        rig.Stats.Apply(Ev.Of($$"""{"type":"milestones.reached","ref":"{{reference}}","milestones":[]}"""));
        Assert.Null(rig.Stats.Pending); // nothing new
        rig.Stats.Apply(Ev.Of($$"""{"type":"milestones.reached","ref":"{{reference}}","milestones":[{"id":"words_1000","kind":"words","threshold":1000,"reached":true},{"id":"streak_7","kind":"streak","threshold":7,"reached":true}]}"""));
        var shown = Assert.IsType<StatsModel.Celebration>(rig.Stats.Pending);
        Assert.Equal("Milestone · 7 active days in a row", shown.Note); // the biggest one, said once
        rig.Stats.DismissCelebration(shown.Serial + 1);
        Assert.NotNull(rig.Stats.Pending); // a later serial is not this one
        rig.Stats.DismissCelebration(shown.Serial);
        Assert.Null(rig.Stats.Pending);
        Assert.Equal("Milestone · 100,000 words dictated", StatsFormat.MilestoneNote(MilestoneKind.Words, 100_000, Gb));
    }

    /// <summary>
    /// The core reports each milestone once ever, so an answer to an earlier check counts as much
    /// as the newest. Two checks in flight, the first finding a milestone and the second nothing,
    /// still celebrate it; a smaller one arriving later does not replace it.
    /// </summary>
    [Fact]
    public void OverlappingChecksNeverLoseAMilestone()
    {
        var rig = new Rig();
        rig.Stats.CheckMilestones();
        var first = rig.LastRef("milestones.check");
        rig.Stats.CheckMilestones();
        var second = rig.LastRef("milestones.check");
        Assert.NotEqual(first, second);
        rig.Stats.Apply(Ev.Of($$"""{"type":"milestones.reached","ref":"{{first}}","milestones":[{"id":"words_10000","kind":"words","threshold":10000,"reached":true}]}"""));
        rig.Stats.Apply(Ev.Of($$"""{"type":"milestones.reached","ref":"{{second}}","milestones":[]}"""));
        Assert.Equal("words_10000", rig.Stats.Pending?.Id);
        rig.Stats.CheckMilestones();
        var third = rig.LastRef("milestones.check");
        rig.Stats.Apply(Ev.Of($$"""{"type":"milestones.reached","ref":"{{third}}","milestones":[{"id":"words_1000","kind":"words","threshold":1000,"reached":true}]}"""));
        Assert.Equal("words_10000", rig.Stats.Pending?.Id); // the biggest stays
        rig.Stats.Apply(Ev.Of("""{"type":"milestones.reached","ref":"elsewhere-1","milestones":[{"id":"streak_100","kind":"streak","threshold":100,"reached":true}]}"""));
        Assert.Equal("words_10000", rig.Stats.Pending?.Id); // not this model's question
    }

    /// <summary>
    /// Quick clicks send 45, 50, 55; the core's echoes of 45 and 50 arrive after the third click
    /// and are not taken over it. The last echo is.
    /// </summary>
    [Fact]
    public void EchoesOfEarlierWritesDoNotStepBack()
    {
        var rig = new Rig();
        var stats = rig.Stats;
        stats.SetTypingWpm(45);
        stats.SetTypingWpm(50);
        stats.SetTypingWpm(55);
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"45"}"""));
        Assert.Equal(55, stats.TypingWpm);
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"50"}"""));
        Assert.Equal(55, stats.TypingWpm);
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"55"}"""));
        Assert.Equal(55, stats.TypingWpm);
        // Nothing of its own in flight: a value from elsewhere is taken.
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"70"}"""));
        Assert.Equal(70, stats.TypingWpm);
        // A failed write is no longer awaited, and the warning clears with the next value.
        stats.SetTypingWpm(75);
        stats.Apply(Ev.Of("""{"type":"command.failed","command":"setting.set","id":"setting:stats.typing_wpm","message":"x"}"""));
        Assert.True(stats.SettingsFailed);
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"70"}"""));
        Assert.Equal(70, stats.TypingWpm);
        Assert.False(stats.SettingsFailed);
        // A write the core never echoed (it started again) does not swallow the new core's answer.
        stats.SetTypingWpm(80);
        stats.Apply(Ev.Of("""{"type":"core.ready","version":"1.0.0","abi":2}"""));
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"65"}"""));
        Assert.Equal(65, stats.TypingWpm);
    }

    /// <summary>The window off screen: nothing is counted for it; back on screen, Stats counts again.</summary>
    [Fact]
    public void AnOffScreenWindowWaitsAndCountsWhenBack()
    {
        var rig = new Rig();
        rig.Stats.ScreenAppeared();
        rig.Stats.WindowPresence(onScreen: false);
        rig.Sent.Commands.Clear();
        rig.Stats.Apply(Ev.Of("""{"type":"dictation.inserted","outcome":"pasted","record":"r1","text":"x"}"""));
        Assert.Equal(["milestones.check"], rig.Names());
        rig.Stats.WindowPresence(onScreen: true);
        Assert.Equal(["milestones.check", "stats.get"], rig.Names());
    }

    /// <summary>
    /// An answer that never comes ends in "couldn't count" after 20 s, not a spinner for ever; a
    /// late limit for an earlier question changes nothing, and an answer cancels its wait.
    /// </summary>
    [Fact]
    public void ALoadThatIsNeverAnsweredFails()
    {
        var rig = new Rig();
        rig.Stats.Reload();
        rig.Stats.Reload();
        Assert.Equal(2, rig.Wakes.Scheduled.Count);
        Assert.All(rig.Wakes.Scheduled, w => Assert.Equal(TimeSpan.FromSeconds(20), w.Delay));
        Assert.True(rig.Wakes.Scheduled[0].Handle.Disposed); // the first question's wait went with it
        rig.Wakes.Scheduled[0].Wake();
        Assert.Equal(StatsModel.Load.Loading, rig.Stats.LoadState);
        rig.Wakes.Scheduled[1].Wake();
        Assert.Equal(StatsModel.Load.Failed, rig.Stats.LoadState);

        var answered = new Rig();
        answered.Stats.Reload();
        answered.Stats.Apply(Ev.Of(Counted(answered.LastRef("stats.get"))));
        Assert.True(answered.Wakes.Scheduled.Single().Handle.Disposed);
        // A refresh that hangs leaves the last numbers up.
        answered.Stats.Reload();
        answered.Wakes.Scheduled[^1].Wake();
        Assert.Equal(StatsModel.Load.Loaded, answered.Stats.LoadState);
    }

    /// <summary>The glow and the line read aloud play once per celebration, even when the window leaves the screen and comes back while the line waits.</summary>
    [Fact]
    public void TheGlowAndTheAnnouncementPlayOnce()
    {
        var stats = new Rig().Stats;
        Assert.True(stats.BeginGlow(1));
        Assert.False(stats.BeginGlow(1));
        Assert.True(stats.BeginGlow(2));
        Assert.True(stats.BeginAnnouncement(1));
        Assert.False(stats.BeginAnnouncement(1));
    }

    [Fact]
    public void TheSettingsAreReadAndSetAndTheSpeedRecountsTimeSaved()
    {
        var rig = new Rig();
        var stats = rig.Stats;
        Assert.True(stats.Celebrate);
        Assert.Equal(40, stats.TypingWpm);
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.celebrate","value":"off"}"""));
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"55"}"""));
        Assert.False(stats.Celebrate);
        Assert.Equal(55, stats.TypingWpm);
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"nonsense"}"""));
        Assert.Equal(40, stats.TypingWpm); // unreadable reads as the default, as the core reads it

        stats.SetCelebrate(true);
        Assert.True(stats.Celebrate);
        stats.SetTypingWpm(500);
        stats.SetTypingWpm(3);
        Assert.Equal(["on", "200", "10"], rig.Sent.Commands.OfType<CoreCommand.SettingSet>().Select(s => s.Value)); // clamped to what the core takes

        // A setting that could not be saved is said in Settings.
        Assert.False(stats.SettingsFailed);
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:stats.celebrate","message":"disk I/O error"}""");
        Assert.True(StatsModel.Handles(failed));
        stats.Apply(failed);
        Assert.True(stats.SettingsFailed);

        // Its own writes' echoes, taken as the core's word.
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.celebrate","value":"on"}"""));
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"200"}"""));
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"10"}"""));
        Assert.Equal(10, stats.TypingWpm);

        // With the screen showing, a new speed recounts.
        stats.ScreenAppeared();
        stats.Apply(Ev.Of(Counted(rig.LastRef("stats.get"))));
        rig.Sent.Commands.Clear();
        stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.typing_wpm","value":"60"}"""));
        Assert.Equal(["stats.get"], rig.Names());
    }

    /// <summary>Settings > Stats says what each setting does, in Windows terms, and where the numbers are counted.</summary>
    [Fact]
    public void SettingsSaysWhatEachSettingDoes()
    {
        Assert.Equal("Celebrate milestones", StatsModel.CelebrateTitle);
        Assert.EndsWith("With Always still or Windows' animation effects off, only the line.", StatsModel.CelebrateDetail, StringComparison.Ordinal);
        Assert.Equal("Time saved is typing the same words at this speed, less the time spent speaking.", StatsModel.TypingDetail);
        Assert.Equal("Counted on this PC from your library. Nothing is sent, and nothing is compared with anyone.", StatsModel.WhereText);
        Assert.Equal("40 words per minute", StatsModel.TypingSpoken(40));
        Assert.DoesNotContain("Mac", StatsModel.CelebrateDetail + StatsModel.WhereText, StringComparison.Ordinal);
    }

    /// <summary>
    /// A celebration waits for the window to be on screen; Always still or animations off mean the
    /// line alone, no glow; the glow is about two and a half seconds (the Mac's MilestoneCelebration).
    /// </summary>
    [Fact]
    public void TheCelebrationWaitsForTheWindowAndStillMeansNoGlow()
    {
        var pending = new StatsModel.Celebration(1, "words_1000", "Milestone · 1,000 words dictated");
        Assert.Null(MilestoneCelebration.Showing(pending, onScreen: false));
        Assert.Equal(pending, MilestoneCelebration.Showing(pending, onScreen: true));
        Assert.Null(MilestoneCelebration.Showing(null, onScreen: true));
        Assert.True(MilestoneCelebration.Glows(still: false, reduceMotion: false));
        Assert.False(MilestoneCelebration.Glows(still: true, reduceMotion: false));
        Assert.False(MilestoneCelebration.Glows(still: false, reduceMotion: true));
        var glow = MilestoneCelebration.GlowIn + MilestoneCelebration.GlowHeld + MilestoneCelebration.GlowOut;
        Assert.Equal(2.6, glow.TotalSeconds, 3);
        Assert.Equal(TimeSpan.FromSeconds(6), MilestoneCelebration.Shown);
        Assert.Equal(0.45, MilestoneCelebration.GlowPeak);
    }

    /// <summary>The hub routes the events to the model and lets it show its own failures.</summary>
    [Fact]
    public void TheScreensCarryStats()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send);
        screens.Apply([Ev.Of("""{"type":"core.ready","version":"1.0.0","abi":2}""")]);
        Assert.Contains(sent.Commands, c => c.Name == "milestones.check");
        Assert.True(screens.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.get","id":"setting:stats.typing_wpm","message":"x"}""")));
    }
}

public class StatsFormatTests
{
    private static readonly CultureInfo Gb = CultureInfo.GetCultureInfo("en-GB");

    [Fact]
    public void TimeSavedAlwaysNamesItsAssumption()
    {
        Assert.Equal("3 h 20 min saved vs typing at 40 wpm", StatsFormat.Saved(12_000_000, 40));
        Assert.Equal("under 1 min saved vs typing at 55 wpm", StatsFormat.Saved(20_000, 55));
        Assert.Equal("No time saved yet vs typing at 40 wpm", StatsFormat.Saved(0, 40));
        Assert.Equal("No time saved yet vs typing at 40 wpm", StatsFormat.Saved(-90_000, 40));
    }

    [Fact]
    public void SpeedIsAgainstYourOwnAverage()
    {
        Assert.Equal("142 wpm this week · your 30-day average 135 wpm", StatsFormat.Speed(142, 135));
        Assert.Equal("142 wpm this week", StatsFormat.Speed(142, null));
        Assert.Equal("No speed this week yet · your 30-day average 135 wpm", StatsFormat.Speed(null, 135));
        Assert.Equal("Your speed shows after a minute of dictation", StatsFormat.Speed(null, null));
    }

    /// <summary>The streak shows from its second day; before that, the longest one, if there was one.</summary>
    [Fact]
    public void TheStreakShowsFromDayTwo()
    {
        Assert.Null(StatsFormat.Streak(0, 0));
        Assert.Null(StatsFormat.Streak(1, 1));
        Assert.Equal("Longest streak: 9 active days", StatsFormat.Streak(1, 9));
        Assert.Equal("Streak: 2 active days", StatsFormat.Streak(2, 2));
        Assert.Equal("Streak: 5 active days · longest 12", StatsFormat.Streak(5, 12));
        Assert.Equal("7 active days in a row", StatsFormat.MilestoneTitle(MilestoneKind.Streak, 7));
        Assert.Equal("1,000 words", StatsFormat.MilestoneTitle(MilestoneKind.Words, 1000, Gb));
    }

    [Fact]
    public void ShortSpansReadInSecondsAndMinutes()
    {
        Assert.Equal("0 s", StatsFormat.Span(0));
        Assert.Equal("45 s", StatsFormat.Span(45_400));
        Assert.Equal("2 min", StatsFormat.Span(120_000));
        Assert.Equal("4 min 10 s", StatsFormat.Span(250_000));
        Assert.Equal("1 h 5 min", StatsFormat.Span(3_900_000));
    }

    [Fact]
    public void TalkTimeSharesAddUpAndPromisesReadAsKeptOfMade()
    {
        var share = StatsFormat.TalkShare(1, 2);
        Assert.Equal((33, 67), share);
        Assert.Null(StatsFormat.TalkShare(0, 0));
        var promises = new PromiseStats { Made = 14, Kept = 12, Open = 1, Overdue = 1 };
        Assert.Equal("Kept 12 of 14 this month", StatsFormat.Kept(promises, "this month"));
        Assert.Equal("1 open · 1 overdue", StatsFormat.PromiseDetail(promises));
        Assert.Equal("Talk time: you 33 percent, 1 s; them 67 percent, 2 s", StatsFormat.TalkSpoken((33, 67), 1_000, 2_000));
        Assert.Equal("You 33 % · 1 s", StatsFormat.TalkKey("You", 33, 1_000));
    }

    /// <summary>The heatmap is shaded against the busiest day, and Narrator hears a summary, not 83 cells.</summary>
    [Fact]
    public void TheHeatmapIsShadedAndSummarised()
    {
        var words = new long[83];
        words[0] = 10;
        words[30] = 1_204;
        words[82] = 600;
        var counted = Ev.Of<StatsCounted>(StatsModelTests.Counted("x", heatmap: words));
        var cells = StatsFormat.Heatmap(counted.Dictation);
        Assert.Equal(83, cells.Count);
        Assert.Equal(1, cells[0].Level);
        Assert.Equal(4, cells[30].Level);
        Assert.Equal(2, cells[82].Level);
        Assert.Equal(0, cells[1].Level);
        Assert.Equal(0, cells[0].Weekday); // 13 July 2026 starts the first column
        Assert.Equal(new DateOnly(2026, 8, 12), cells[30].Date);
        Assert.Equal(
            "Words dictated per day over the last 12 weeks: 3 of 83 days with dictation. The most was 1,204 words, on Wed 12 Aug.",
            StatsFormat.HeatmapSummary(cells, Gb));
        var empty = StatsFormat.Heatmap(Ev.Of<StatsCounted>(StatsModelTests.Counted("x")).Dictation);
        Assert.Equal("Words dictated per day over the last 12 weeks: no dictation yet.", StatsFormat.HeatmapSummary(empty, Gb));
    }

    /// <summary>The share card carries only the ticked numbers the library has, in its own order (the Mac's ShareCard tests).</summary>
    [Fact]
    public void TheCardCarriesOnlyTheTickedNumbersInAFixedOrder()
    {
        var counted = Ev.Of<StatsCounted>(StatsModelTests.Counted(
            "x", words: (10, 1_200, 12_345), dictations: 30, wpmWeek: 142, savedAll: 12_000_000, streak: 5, longest: 12,
            meetings: """{"meetings":2,"recorded_ms":5400000,"you_ms":600000,"them_ms":1200000,"longest_monologue_ms":90000,"questions":3}""",
            promises: """{"made":14,"kept":12,"open":1,"overdue":1}"""));
        Assert.Equal(
            [new("12,345", "words dictated"), new("3 h 20 min", "saved vs typing at 40 wpm"), new("5 active days", "dictation streak")],
            ShareStats.Lines(counted, ShareStats.Defaults, Gb));
        var every = ShareStats.Lines(counted, ShareStats.All.ToHashSet(), Gb);
        Assert.Equal(
            ["words dictated", "words this week", "speaking speed this week", "saved vs typing at 40 wpm", "dictation streak",
             "in meetings this month", "talk time this month, you / them", "promises kept this month"],
            every.Select(l => l.Label));
        Assert.Equal(["12,345", "1,200", "142 wpm", "3 h 20 min", "5 active days", "1 h 30 min", "33 % / 67 %", "12 of 14"], every.Select(l => l.Value));
        Assert.Equal("The card: 12,345 words dictated", ShareStats.Spoken([every[0]]));
        Assert.Equal("The card is empty", ShareStats.Spoken([]));
    }

    /// <summary>A number the library has not got yet has no line, and its tick says so.</summary>
    [Fact]
    public void ANumberNotYetThereHasNoLine()
    {
        var empty = Ev.Of<StatsCounted>(StatsModelTests.Counted("x"));
        Assert.Empty(ShareStats.Lines(empty, ShareStats.All.ToHashSet(), Gb));
        Assert.All(ShareStats.All, s => Assert.False(s.Available(empty)));
        Assert.Equal("Streak (none yet)", ShareStat.Streak.TickTitle(empty));
        var longestOnly = Ev.Of<StatsCounted>(StatsModelTests.Counted("x", dictations: 3, streak: 1, longest: 4));
        Assert.Equal(new ShareLine("4 active days", "longest dictation streak"), ShareStat.Streak.Line(longestOnly));
    }
}
