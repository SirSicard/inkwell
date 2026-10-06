// The Stats screen's five additions, as the Mac's StatsPlayTests: personal records (and a best's
// note in the Drop), the gentle streak (rest days, a pause, hiding it), last week's review until it
// is dismissed, what time saved is about, and the share card's records, seals and heatmap. What
// the model sends and keeps, how each reads, and that the cards' measures fit the 720-epx window.
// The counting is the core's (ink-ffi's stats tests); nothing here counts.
using System.Globalization;
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class StatsPlayTests
{
    private static readonly CultureInfo Gb = CultureInfo.GetCultureInfo("en-GB");
    private static readonly DateTimeOffset Now = DateTimeOffset.FromUnixTimeSeconds(1_791_028_800);

    /// <summary>Every best, as the core lists them.</summary>
    internal const string AllBests = ""","bests":[{"id":"longest_dictation","unit":"ms","value":192000,"date":"2026-10-01","record":"r1"},{"id":"fastest_dictation","unit":"wpm","value":168,"date":"2026-09-30","record":"r2"},{"id":"most_words_day","unit":"words","value":2340,"date":"2026-09-29"},{"id":"best_week","unit":"words","value":8120,"date":"2026-09-28"},{"id":"longest_meeting","unit":"ms","value":5520000,"date":"2026-09-24","record":"m1"},{"id":"longest_monologue","unit":"ms","value":250000,"date":"2026-09-24","record":"m1"}]""";

    /// <summary>Last week with everything in it: a faster week, time saved, meetings and promises kept.</summary>
    internal const string FullReview = ""","week_review":{"week":"2026-09-28","words":8120,"saved_ms":7800000,"saved_about":[{"key":"feature_film","count":1},{"key":"lunch_hour","count":2}],"best_day":"2026-09-29","best_day_words":2340,"wpm":142,"wpm_gain":6,"meetings":3,"meeting_ms":7500000,"promises_kept":2}""";

    /// <summary>The dictation fields the gentle streak and time pictured add.</summary>
    internal const string PlayDictation = ""","saved_about_week":[{"key":"coffee_break","count":2}],"saved_about_all":[{"key":"working_day","count":2},{"key":"feature_film","count":5}],"latest_streak_days":12,"active_days_month":3,"rest_days":[6,7],"streak_paused_since":"2026-10-02" """;

    private sealed class Wakes : IWakeScheduler
    {
        public List<(TimeSpan Delay, Action Wake)> Scheduled { get; } = [];

        public IDisposable After(TimeSpan delay, Action wake)
        {
            Scheduled.Add((delay, wake));
            return new Handle();
        }

        private sealed class Handle : IDisposable
        {
            public void Dispose()
            {
            }
        }
    }

    private sealed class Rig
    {
        public Sent Sent { get; } = new();
        public Wakes Wakes { get; } = new();
        public StatsModel Stats { get; }

        public Rig() => Stats = new StatsModel(Sent.Send, Wakes, () => TimeZoneInfo.Utc, Gb, () => Now);

        public string LastRef(string cmd) => Sent.Commands.Last(c => c.Name == cmd).CommandId!;

        public List<string> SetValues(string key) =>
            Sent.Commands.OfType<CoreCommand.SettingSet>().Where(c => c.Key.Key() == key).Select(c => c.Value).ToList();
    }

    private static StatsCounted Counted(string json) => Ev.Of<StatsCounted>(json);

    private static TimeEquivalent Picture(TimeEquivalentKey key, long count) => new() { Key = key, Count = count };

    // The model

    /// <summary>Rest days are written as the core reads them, ascending; all seven never are.</summary>
    [Fact]
    public void RestDaysAreWrittenAscendingAndNeverAllSeven()
    {
        var rig = new Rig();
        rig.Stats.SetRestDay(7, true);
        rig.Stats.SetRestDay(6, true);
        Assert.Equal([6, 7], rig.Stats.RestDays.Order());
        Assert.Equal(["7", "6,7"], rig.SetValues("stats.rest_days"));
        for (var day = 1; day <= 5; day++)
        {
            rig.Stats.SetRestDay(day, true);
        }
        Assert.Equal([1, 2, 3, 4, 6, 7], rig.Stats.RestDays.Order()); // the seventh (Friday) is refused
        Assert.Equal(6, rig.SetValues("stats.rest_days").Count);
        rig.Stats.SetRestDay(9, true);
        Assert.Equal(6, rig.SetValues("stats.rest_days").Count); // no weekday nine
        for (var day = 1; day <= 7; day++)
        {
            rig.Stats.SetRestDay(day, false);
        }
        Assert.Empty(rig.Stats.RestDays);
        Assert.Equal("none", rig.SetValues("stats.rest_days")[^1]);

        Assert.Equal([6, 7], StatsModel.ParseRestDays("6,7").Order());
        Assert.Empty(StatsModel.ParseRestDays("none"));
        Assert.Empty(StatsModel.ParseRestDays(null));
        foreach (var odd in new[] { "7,6", "6,6", "1,2,3,4,5,6,7", "8", "6,", "x", "" })
        {
            Assert.Empty(StatsModel.ParseRestDays(odd));
        }
        Assert.Equal("1,3,7", StatsModel.RestDaysValue([7, 1, 3]));
    }

    /// <summary>The streak settings are read from the core's echoes; showing Stats counts again when they change what is counted.</summary>
    [Fact]
    public void TheStreakSettingsAreReadAndRecount()
    {
        var rig = new Rig();
        rig.Stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.rest_days","value":"6,7"}"""));
        rig.Stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.streak","value":"hidden"}"""));
        rig.Stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.share_heatmap","value":"on"}"""));
        Assert.Equal([6, 7], rig.Stats.RestDays.Order());
        Assert.False(rig.Stats.StreakShown);
        Assert.True(rig.Stats.ShareHeatmap);
        rig.Stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.share_heatmap","value":"nonsense"}"""));
        Assert.False(rig.Stats.ShareHeatmap); // off unless on

        rig.Stats.SetStreakShown(false);
        rig.Stats.SetShareHeatmap(true);
        Assert.Equal(["hidden"], rig.SetValues("stats.streak"));
        Assert.Equal(["on"], rig.SetValues("stats.share_heatmap"));

        rig.Stats.ScreenAppeared();
        rig.Stats.Apply(Ev.Of(StatsModelTests.Counted(rig.LastRef("stats.get"), dictationExtra: ""","rest_days":[6,7],"streak_hidden":false""")));
        rig.Sent.Commands.Clear();
        rig.Stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.streak","value":"hidden"}"""));
        Assert.Equal(["stats.get"], rig.Sent.Commands.Select(c => c.Name));
        rig.Sent.Commands.Clear();
        rig.Stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.rest_days","value":"6,7"}"""));
        Assert.Empty(rig.Sent.Commands); // the same rest days: nothing to count again
        rig.Stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.rest_days","value":"7"}"""));
        Assert.Equal(["stats.get"], rig.Sent.Commands.Select(c => c.Name));
        rig.Stats.Apply(Ev.Of("""{"type":"command.failed","command":"setting.set","id":"setting:stats.rest_days","message":"x"}"""));
        Assert.True(rig.Stats.SettingsFailed);
    }

    /// <summary>A pause carries the user's calendar and is answered with the numbers; one at a time; a failure or no answer is said.</summary>
    [Fact]
    public void APauseIsAnsweredWithTheNumbersAndAFailureIsSaid()
    {
        var rig = new Rig();
        rig.Stats.PauseStreak();
        var asked = JsonDocument.Parse(rig.Sent.Commands[^1].Json).RootElement;
        Assert.Equal("streak.pause", asked.GetProperty("cmd").GetString());
        Assert.Equal(1, asked.GetProperty("week_start").GetInt32());
        Assert.Equal(JsonValueKind.Array, asked.GetProperty("utc_offsets").ValueKind);
        var reference = asked.GetProperty("id").GetString()!;
        Assert.Equal(StatsModel.StreakChange.Pausing, rig.Stats.StreakChanging);
        rig.Stats.ResumeStreak();
        Assert.Single(rig.Sent.Commands); // one change at a time

        rig.Stats.Apply(Ev.Of(StatsModelTests.Counted(reference, dictations: 3, dictationExtra: ""","streak_paused_since":"2026-10-03" """)));
        Assert.Null(rig.Stats.StreakChanging);
        Assert.Equal("2026-10-03", rig.Stats.Counted?.Dictation.StreakPausedSince);

        rig.Stats.ResumeStreak();
        var failed = Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"streak.resume","id":"{{rig.LastRef("streak.resume")}}","message":"x"}""");
        Assert.True(StatsModel.Handles(failed));
        rig.Stats.Apply(failed);
        Assert.Null(rig.Stats.StreakChanging);
        Assert.Equal(StatsModel.StreakChange.Resuming, rig.Stats.StreakChangeFailed);
        Assert.Equal("2026-10-03", rig.Stats.Counted?.Dictation.StreakPausedSince); // still paused

        // Never answered: said once the time limit passes.
        rig.Stats.PauseStreak();
        Assert.Null(rig.Stats.StreakChangeFailed); // a new try clears the old failure
        var pending = rig.LastRef("streak.pause");
        Assert.Equal(StatsModel.LoadLimit, rig.Wakes.Scheduled[^1].Delay);
        rig.Stats.StreakTimedOut("streak-0");
        Assert.Equal(StatsModel.StreakChange.Pausing, rig.Stats.StreakChanging);
        rig.Wakes.Scheduled[^1].Wake();
        Assert.Equal(StatsModel.StreakChange.Pausing, rig.Stats.StreakChangeFailed);
        // Answered after all: what it did shows, and the failure goes.
        rig.Stats.Apply(Ev.Of(StatsModelTests.Counted(pending, dictations: 3, dictationExtra: ""","streak_paused_since":"2026-10-03" """)));
        Assert.Null(rig.Stats.StreakChangeFailed);
    }

    /// <summary>A pause never answered while the screen has nothing to show ends in "couldn't count".</summary>
    [Fact]
    public void APauseNeverAnsweredDoesNotSpinTheScreenForEver()
    {
        var rig = new Rig();
        rig.Stats.Reload();
        var get = rig.LastRef("stats.get");
        rig.Stats.PauseStreak();
        rig.Stats.LoadTimedOut(get);
        Assert.Equal(StatsModel.Load.Loading, rig.Stats.LoadState);
        rig.Stats.StreakTimedOut(rig.LastRef("streak.pause"));
        Assert.Equal(StatsModel.Load.Failed, rig.Stats.LoadState);
    }

    /// <summary>A stats.get sent before a pause and answered after it would show the numbers from before: only the pause's answer is taken.</summary>
    [Fact]
    public void AnEarlierCountDoesNotUndoAPause()
    {
        var rig = new Rig();
        rig.Stats.Reload();
        var get = rig.LastRef("stats.get");
        rig.Stats.PauseStreak();
        var pause = rig.LastRef("streak.pause");
        rig.Stats.Apply(Ev.Of(StatsModelTests.Counted(get, dictations: 1)));
        Assert.Null(rig.Stats.Counted);
        rig.Stats.Apply(Ev.Of(StatsModelTests.Counted(pause, dictations: 1, dictationExtra: ""","streak_paused_since":"2026-10-03" """)));
        Assert.Equal("2026-10-03", rig.Stats.Counted?.Dictation.StreakPausedSince);
        Assert.Equal(StatsModel.Load.Loaded, rig.Stats.LoadState);
    }

    /// <summary>Settings counts once, the first time it shows, so the pause row knows whether one runs.</summary>
    [Fact]
    public void SettingsCountsOnceWhenItFirstShows()
    {
        var rig = new Rig();
        rig.Stats.SettingsAppeared();
        rig.Stats.SettingsAppeared();
        Assert.Equal(["stats.get"], rig.Sent.Commands.Select(c => c.Name));
        Assert.Equal("Counting…", StatsModel.PauseUnknown(failed: false));
        Assert.Equal("Couldn't read whether the streak is paused.", StatsModel.PauseUnknown(failed: true));
    }

    /// <summary>Last week's review stays until dismissed; a dismissal not saved brings it back and says so, for that week only.</summary>
    [Fact]
    public void TheReviewStaysUntilDismissedAndComesBackIfNotSaved()
    {
        var rig = new Rig();
        void Answer(string review)
        {
            rig.Stats.Reload();
            rig.Stats.Apply(Ev.Of(StatsModelTests.Counted(rig.LastRef("stats.get"), (1, 2, 3), 2, extra: review)));
        }
        Answer(FullReview);
        var review = Assert.IsType<WeekReview>(rig.Stats.WeekReview);
        Assert.Equal("2026-09-28", review.Week);
        rig.Stats.DismissReview(review);
        Assert.Null(rig.Stats.WeekReview);
        Assert.Equal(["2026-09-28"], rig.SetValues("stats.review_dismissed"));
        rig.Stats.Apply(Ev.Of("""{"type":"command.failed","command":"setting.set","id":"setting:stats.review_dismissed","message":"x"}"""));
        Assert.Equal(review, rig.Stats.WeekReview);
        Assert.True(rig.Stats.ReviewDismissFailed);
        Assert.False(rig.Stats.SettingsFailed); // said on the card, not in Settings

        Answer(FullReview.Replace("2026-09-28", "2026-10-05", StringComparison.Ordinal));
        Assert.False(rig.Stats.ReviewDismissFailed); // a new week's review was never dismissed
        Answer(FullReview);
        Assert.True(rig.Stats.ReviewDismissFailed);

        rig.Stats.DismissReview(rig.Stats.WeekReview!);
        Assert.False(rig.Stats.ReviewDismissFailed);
        rig.Stats.Apply(Ev.Of("""{"type":"setting.value","key":"stats.review_dismissed","value":"2026-09-28"}"""));
        Assert.Null(rig.Stats.WeekReview);
    }

    /// <summary>A best is a plain note in the Drop that never replaces one showing (an alert above all); a milestone keeps its line, by its name.</summary>
    [Fact]
    public void ABestIsANoteInTheDropThatYields()
    {
        const string Best = """{"type":"milestones.reached","ref":"milestones-4","milestones":[],"best":{"id":"longest_dictation","unit":"ms","old":160000,"new":192000,"date":"2026-10-03","record":"r9"}}""";
        var note = DictationDrop.Note(Ev.Of(Best), hasLanguageModel: true);
        Assert.Equal(new DropLine("Longest dictation yet", "3 min 12 s · previous best 2 min 40 s", Yields: true), note);
        Assert.Null(DictationDrop.Note(Ev.Of("""{"type":"milestones.reached","ref":"milestones-5","milestones":[]}"""), hasLanguageModel: true));

        var store = new CoreStore();
        var drop = new DropModel(new Wakes());
        void Apply(string json)
        {
            var batch = new List<InkEvent> { Ev.Of(json) };
            store.Apply(batch);
            drop.Apply(store, batch);
        }
        Apply(Best);
        Assert.Equal("Longest dictation yet", drop.Line?.Title);

        var blocked = new CoreStore();
        var later = new DropModel(new Wakes());
        var take = new List<InkEvent> { Ev.Of("""{"type":"dictation.inserted","outcome":"blocked","text":"x"}""") };
        blocked.Apply(take);
        later.Apply(blocked, take);
        var best = new List<InkEvent> { Ev.Of(Best) };
        blocked.Apply(best);
        later.Apply(blocked, best);
        Assert.Equal(DropLineTone.Alert, later.Line?.Tone); // the alert stays

        var rig = new Rig();
        rig.Stats.CheckMilestones();
        rig.Stats.Apply(Ev.Of($$"""{"type":"milestones.reached","ref":"{{rig.LastRef("milestones.check")}}","milestones":[{"id":"words_10000","kind":"words","threshold":10000,"reached":true,"name":"notebook"}]}"""));
        Assert.Equal("A notebook · 10,000 words dictated", rig.Stats.Pending?.Note);
    }

    // The words

    /// <summary>Each of the core's pictures of time, one and many; always "about"; nothing when the core has none.</summary>
    [Fact]
    public void TimeSavedIsPicturedPlainly()
    {
        var one = new Dictionary<TimeEquivalentKey, string>
        {
            [TimeEquivalentKey.WorkingWeek] = "a working week",
            [TimeEquivalentKey.WorkingDay] = "a working day",
            [TimeEquivalentKey.FeatureFilm] = "a feature film",
            [TimeEquivalentKey.LunchHour] = "a lunch hour",
            [TimeEquivalentKey.CoffeeBreak] = "a coffee break",
        };
        Assert.Equal(Enum.GetValues<TimeEquivalentKey>().Length, one.Count); // every key
        foreach (var (key, words) in one)
        {
            Assert.Equal(words, StatsFormat.Equivalent(key, 1));
            Assert.DoesNotContain("!", StatsFormat.Equivalent(key, 2), StringComparison.Ordinal);
        }
        Assert.Equal("12 working weeks", StatsFormat.Equivalent(TimeEquivalentKey.WorkingWeek, 12));
        Assert.Equal("3 coffee breaks", StatsFormat.Equivalent(TimeEquivalentKey.CoffeeBreak, 3));
        IReadOnlyList<TimeEquivalent> film = [Picture(TimeEquivalentKey.FeatureFilm, 2), Picture(TimeEquivalentKey.LunchHour, 4)];
        Assert.Equal("about 2 feature films", StatsFormat.About(film)); // the largest first
        Assert.Equal("That's about 2 feature films.", StatsFormat.SavedAbout(film));
        Assert.Null(StatsFormat.About(null));
        Assert.Null(StatsFormat.About([]));
        Assert.Equal("This week: 25 min, about 2 coffee breaks", StatsFormat.SavedThisWeek(1_500_000, [Picture(TimeEquivalentKey.CoffeeBreak, 2)]));
        Assert.Equal("This week: 10 min", StatsFormat.SavedThisWeek(600_000, null));
    }

    [Fact]
    public void RecordsReadWithTheirDay()
    {
        var c = Counted(StatsModelTests.Counted("x", dictations: 9, extra: AllBests));
        var records = StatsFormat.Records(c.Bests, Gb);
        Assert.Equal(["3 min 12 s", "168 wpm", "2,340 words", "8,120 words", "1 h 32 min", "4 min 10 s"], records.Select(r => r.Value));
        string On(string date) => StatsFormat.ShortDate(date, Gb);
        Assert.Equal("1 Oct", On("2026-10-01"));
        Assert.Equal(
            [
                "longest dictation · 1 Oct", $"fastest dictation · {On("2026-09-30")}", $"most words in a day · {On("2026-09-29")}",
                $"most words in a week · week of {On("2026-09-28")}", $"longest meeting · {On("2026-09-24")}", $"longest monologue · {On("2026-09-24")}",
            ],
            records.Select(r => r.Label));
        Assert.Equal("Longest dictation: 3 min 12 s, on 1 Oct", records[0].Spoken);
        Assert.Equal($"Most words in a week: 8,120 words, the week of {On("2026-09-28")}", records[3].Spoken);
        Assert.Empty(StatsFormat.Records(null, Gb));
        Assert.Contains("30 seconds", StatsFormat.RecordsRule, StringComparison.Ordinal);
        Assert.Contains("this PC", StatsFormat.RecordsRule, StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("longest_dictation", "ms", 160_000, 192_000, "Longest dictation yet", "3 min 12 s · previous best 2 min 40 s")]
    [InlineData("fastest_dictation", "wpm", 150, 168, "Fastest dictation yet", "168 wpm · previous best 150 wpm")]
    [InlineData("most_words_day", "words", 1_980, 2_340, "Most words in a day yet", "2,340 words · previous best 1,980 words")]
    [InlineData("best_week", "words", 7_400, 8_120, "Most words in a week yet", "8,120 words · previous best 7,400 words")]
    [InlineData("longest_meeting", "ms", 4_200_000, 5_520_000, "Longest meeting yet", "1 h 32 min · previous best 1 h 10 min")]
    [InlineData("longest_monologue", "ms", 185_000, 250_000, "Longest monologue yet", "4 min 10 s · previous best 3 min 5 s")]
    public void EachBestHasItsNote(string id, string unit, long old, long @new, string title, string detail)
    {
        var news = JsonSerializer.Deserialize<BestNews>($$"""{"id":"{{id}}","unit":"{{unit}}","old":{{old}},"new":{{@new}},"date":"2026-10-03"}""")!;
        var note = StatsFormat.BestNote(news, Gb);
        Assert.Equal(title, note.Title);
        Assert.Equal(detail, note.Detail);
        Assert.True(note.Yields);
        Assert.Equal(DropLineTone.Plain, note.Tone);
    }

    /// <summary>The review says gains and plain facts; a slower week says only its speed.</summary>
    [Fact]
    public void TheReviewSaysGainsAndPlainFactsOnly()
    {
        var review = Counted(StatsModelTests.Counted("x", dictations: 9, extra: FullReview)).WeekReview!;
        Assert.StartsWith("Week of 28 ", StatsFormat.ReviewWeek(review, Gb), StringComparison.Ordinal);
        Assert.Equal(["8,120", "2 h 10 min", "2 h 5 min"], StatsFormat.ReviewNumbers(review, Gb).Select(n => n.Value));
        Assert.Equal(["words", "saved", "in 3 meetings"], StatsFormat.ReviewNumbers(review, Gb).Select(n => n.Label));
        Assert.Equal(
            [
                "Busiest day: Tuesday, 2,340 words",
                "142 wpm, 6 faster than the four weeks before",
                "Time saved: about a feature film",
                "2 promises from its meetings kept",
            ],
            StatsFormat.ReviewLines(review, Gb));
        var plain = Counted(StatsModelTests.Counted("x", dictations: 9, extra: ""","week_review":{"week":"2026-09-28","words":1,"wpm":120,"meetings":0,"meeting_ms":0}""")).WeekReview!;
        Assert.Equal(["120 wpm"], StatsFormat.ReviewLines(plain, Gb));
        Assert.Equal(["word"], StatsFormat.ReviewNumbers(plain, Gb).Select(n => n.Label));
        foreach (var line in StatsFormat.ReviewLines(review, Gb).Concat(StatsFormat.ReviewLines(plain, Gb)))
        {
            foreach (var word in new[] { "down", "slower", "fewer", "less", "lost", "!" })
            {
                Assert.DoesNotContain(word, line, StringComparison.OrdinalIgnoreCase);
            }
        }
    }

    /// <summary>An ended streak is shown by what it reached and the longest, never as lost; a pause and the rest days are said; a hidden streak says nothing.</summary>
    [Fact]
    public void TheStreakIsGentle()
    {
        var ended = Counted(StatsModelTests.Counted("x", dictations: 9, streak: 0, longest: 23, dictationExtra: PlayDictation)).Dictation;
        Assert.Equal("Latest streak: 12 active days · longest 23", StatsFormat.Streak(ended));
        Assert.Equal("3 active days this month", StatsFormat.ActiveDaysThisMonth(ended));
        Assert.Equal(StatsFormat.StreakRule + " Rest days (Sat, Sun) neither count nor break it.", StatsFormat.StreakRuleWith([6, 7], Gb));
        Assert.Equal(StatsFormat.StreakRule, StatsFormat.StreakRuleWith([], Gb));
        Assert.StartsWith("Paused since 2 Oct", StatsFormat.Paused("2026-10-02", Gb), StringComparison.Ordinal);
        var running = Counted(StatsModelTests.Counted("x", dictations: 9, streak: 5, longest: 23, dictationExtra: PlayDictation)).Dictation;
        Assert.Equal("Streak: 5 active days · longest 23", StatsFormat.Streak(running));
        Assert.Null(StatsFormat.ActiveDaysThisMonth(running));
        var hidden = Counted(StatsModelTests.Counted("x", dictations: 9, streak: 5, longest: 23, dictationExtra: ""","streak_hidden":true""")).Dictation;
        Assert.Null(StatsFormat.Streak(hidden));
        Assert.Null(StatsFormat.ActiveDaysThisMonth(hidden));
        Assert.DoesNotContain("lost", StatsFormat.Streak(ended)!, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void WeekdaysFollowTheUsersWeek()
    {
        Assert.Equal([1, 2, 3, 4, 5, 6, 7], StatsFormat.Weekdays(Gb).Select(w => w.Iso));
        Assert.Equal("Monday", StatsFormat.Weekdays(Gb)[0].Name);
        var us = CultureInfo.GetCultureInfo("en-US");
        Assert.Equal([7, 1, 2, 3, 4, 5, 6], StatsFormat.Weekdays(us).Select(w => w.Iso));
        Assert.Equal("Sun", StatsFormat.Weekdays(us)[0].Abbreviated);
        Assert.Equal("Saturday, rest day", StatsModel.RestDaySpoken(StatsFormat.Weekdays(Gb)[5], rest: true));
    }

    [Fact]
    public void MilestonesHaveNames()
    {
        Assert.Equal(
            ["First page", "A notebook", "A short novel", "A novel's worth", "Seven-day run", "Thirty-day run", "Hundred-day run"],
            Enum.GetValues<MilestoneName>().Select(StatsFormat.MilestoneNamed));
        var c = Counted(StatsModelTests.Counted("x", reached: new HashSet<string> { "words_1000" }, named: true));
        Assert.Equal("First page · 1,000 words", StatsFormat.MilestoneChip(c.Milestones[0], Gb));
        Assert.Equal("First page · 1,000 words, reached", StatsFormat.MilestoneSpoken(c.Milestones[0], Gb));
        Assert.Equal("A short novel · 50,000 words dictated", StatsFormat.MilestoneNote(MilestoneKind.Words, 50_000, Gb, MilestoneName.ShortNovel));
        Assert.Equal("Milestone · 7 active days in a row", StatsFormat.MilestoneNote(MilestoneKind.Streak, 7));
        Assert.Equal(["1k", "100k", "30"], new[] { StatsFormat.SealMark(MilestoneKind.Words, 1000), StatsFormat.SealMark(MilestoneKind.Words, 100_000), StatsFormat.SealMark(MilestoneKind.Streak, 30) });
    }

    [Fact]
    public void SettingsWordsAreThePCs()
    {
        foreach (var words in new[] { StatsModel.StreakDetail, StatsModel.RestDaysDetail, StatsModel.PauseDetail, StatsFormat.RecordsRule, StatsModel.CelebrateDetail })
        {
            Assert.DoesNotContain("Mac", words, StringComparison.Ordinal);
            Assert.DoesNotContain("!", words, StringComparison.Ordinal);
        }
        Assert.StartsWith("Paused since 2 Oct. It ends by itself after 90 days", StatsModel.PauseCaption("2026-10-02", Gb), StringComparison.Ordinal);
        Assert.Equal(StatsModel.PauseDetail, StatsModel.PauseCaption(null, Gb));
        Assert.Equal("Couldn't pause the streak. Try again.", StatsModel.StreakFailure(StatsModel.StreakChange.Pausing));
    }

    // The share card

    [Fact]
    public void TheCardCarriesRecordsSealsAndTheHeatmapOnlyWhenAsked()
    {
        var heat = new long[83];
        heat[80] = 200;
        var c = Counted(StatsModelTests.Counted(
            "x", (10, 20, 12_000), 40, streak: 9, longest: 9, heatmap: heat,
            reached: new HashSet<string> { "words_1000", "words_10000", "streak_7" }, named: true, extra: AllBests));
        var seals = ShareSeal.Available(c);
        Assert.Equal(["First page", "A notebook", "Seven-day run"], seals.Select(s => s.Name));
        Assert.Equal(["1k", "10k", "7"], seals.Select(s => s.Mark));
        var records = ShareRecords.Line(c, Gb)!;
        Assert.Equal("Longest dictation: 3 min 12 s · Fastest dictation: 168 wpm · Most words in a day: 2,340", records.Replace(' ', ' '));
        Assert.Equal(5, records.Split(' ').Length); // it breaks only between records

        var plain = ShareContent.Make(c, new HashSet<ShareStat> { ShareStat.WordsAll }, records: false, heatmap: false, new HashSet<string>(), Gb);
        Assert.Null(plain.Records);
        Assert.Null(plain.Heatmap); // off unless the user turned it on
        Assert.Empty(plain.Seals);
        var full = ShareContent.Make(c, new HashSet<ShareStat> { ShareStat.WordsAll }, records: true, heatmap: true, new HashSet<string> { "words_10000", "streak_100" }, Gb);
        Assert.NotNull(full.Records);
        Assert.Equal(83, full.Heatmap?.Count);
        Assert.Equal(4, full.Heatmap?[80]);
        Assert.Equal(["words_10000"], full.Seals.Select(s => s.Id)); // only seals reached
        Assert.Contains("seals: A notebook", full.Spoken(), StringComparison.Ordinal);
        Assert.Contains("heatmap", full.Spoken(), StringComparison.Ordinal);
        Assert.Equal("The card is empty", ShareContent.Empty.Spoken());
        Assert.False((ShareContent.Empty with { Seals = seals }).IsEmpty); // a seal alone is a card

        var none = Counted(StatsModelTests.Counted("x"));
        Assert.Null(ShareContent.Make(none, new HashSet<ShareStat>(), records: true, heatmap: true, new HashSet<string>(), Gb).Heatmap);
        Assert.Null(ShareRecords.Line(none, Gb));
    }

    [Fact]
    public void AHiddenStreakIsOfferedNowhere()
    {
        var c = Counted(StatsModelTests.Counted("x", (1, 2, 3), 2, streak: 9, longest: 9, dictationExtra: ""","streak_hidden":true"""));
        Assert.False(ShareStat.Streak.Available(c));
        Assert.DoesNotContain(ShareStats.Lines(c, new HashSet<ShareStat> { ShareStat.Streak }, Gb), l => l.Label.Contains("streak", StringComparison.Ordinal));
    }

    // The 720-epx window

    /// <summary>
    /// A Stats card's content at the 720-epx window (less the navigation pane, the page's padding
    /// and the card's): two records side by side, each at least its least width; at 1040, four.
    /// </summary>
    [Fact]
    public void TheRecordsFitTheNarrowestWindowInEvenColumns()
    {
        var narrow = StatsLayout.CardContent(StatsLayout.MinimumWindow);
        Assert.Equal(358, narrow);
        var (count, width) = StatsLayout.Columns(narrow, StatsLayout.RecordMinimum, StatsLayout.RecordSpacing, 6);
        Assert.Equal(2, count);
        Assert.True(width >= StatsLayout.RecordMinimum);
        Assert.True(count * width + (count - 1) * StatsLayout.RecordSpacing <= narrow + 0.5);
        Assert.Equal(4, StatsLayout.Columns(StatsLayout.CardContent(1040), StatsLayout.RecordMinimum, StatsLayout.RecordSpacing, 6).Count);
        Assert.Equal(2, StatsLayout.Columns(2000, StatsLayout.RecordMinimum, StatsLayout.RecordSpacing, 2).Count); // no empty columns
        Assert.Equal((1, 100), StatsLayout.Columns(100, StatsLayout.RecordMinimum, StatsLayout.RecordSpacing, 6)); // one, as wide as there is
        for (double w = 100; w <= 900; w += 7)
        {
            var (n, cw) = StatsLayout.Columns(w, 150, 24, 6);
            Assert.True(n * cw + (n - 1) * 24 <= w + 0.5, $"{w}");
            Assert.True(n == 1 || cw >= 150, $"{w}");
        }
        // The review's numbers and the heatmap's 12 weeks of 12 + 3 fit the same width.
        Assert.True(12 * 15 <= narrow);
        // The rest days wrap in the narrowest Settings column, each button whole.
        var buttons = Enumerable.Repeat((44.0, 32.0), 7).ToList();
        var (places, wrapped, _) = WrapLayout.Place(buttons, 186, 6, 6);
        Assert.True(wrapped <= 186);
        Assert.Equal(7, places.Count);
    }
}
