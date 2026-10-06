// Stats, as the Mac's StatsModel: what the library says about the user's dictation and meetings,
// counted by the core on this PC (stats.get), and the milestones it celebrates once each
// (milestones.check). Nothing is sent anywhere, compared with anyone, or drawn from what was said;
// the core's stats module says where each number comes from.
//
// Days are the user's: each question carries their time zone's UTC offsets over the last ten
// years (each from the moment it took effect, so a record made before a daylight-saving change
// keeps its day) and the weekday their culture starts weeks on.
//
// Milestones are checked when the core is ready (its first check notes what is already reached,
// so nothing old is celebrated) and whenever a dictation is saved or a meeting ends. The core
// reports each milestone once ever, so every check's answer counts, not only the newest: two checks
// in flight would otherwise lose one. A milestone reached waits here until the window is on screen,
// where the window shows its glow and note, each once.
//
// The days are the user's current zone's, read afresh for each question (as the Mac's
// autoupdating calendar is): a PC that changes zone while Inkwell runs counts in the new one. A
// record made while travelling is placed by the zone the PC is in now, as the store keeps no zone
// per record.
//
// The gentle streak (Settings > Stats), as the Mac's: rest days, a pause and a switch to hide it,
// all the core's to count; this model keeps what the user chose and sends it. A pause or resume is
// answered with the numbers counted afresh. Last week's review leads the screen until the user
// dismisses it: the dismissal is the core's to keep (stats.review_dismissed), and the card goes at
// once, coming back only if the dismissal could not be saved. A best just set is the Drop's to say
// (DictationDrop.Note): the core reports it once, in a milestone check's answer.
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class StatsModel : ObservableModel
{
    /// <summary>Where the numbers are: asked and not answered, answered, or could not be counted.</summary>
    public enum Load
    {
        Idle,
        Loading,
        Loaded,
        Failed,
    }

    /// <summary>A milestone reached, to celebrate once: a quiet glow on the orb and one line.</summary>
    /// <param name="Serial">Increases with each, so a later one replaces the note and restarts its time.</param>
    public sealed record Celebration(int Serial, string Id, string Note);

    /// <summary>The core's default typing speed, and the range and step it takes (ink-ffi's stats module).</summary>
    public const int DefaultTypingWpm = 40;
    public const int MinTypingWpm = 10;
    public const int MaxTypingWpm = 200;
    public const int TypingWpmStep = 5;

    /// <summary>How far back the zone's offsets reach.</summary>
    public const int OffsetYears = 10;

    /// <summary>How long the screen waits for its numbers before it says it couldn't count them.</summary>
    public static readonly TimeSpan LoadLimit = TimeSpan.FromSeconds(20);

    /// <summary>
    /// The milestones, smallest first within each kind and words before streaks: the core's fixed
    /// order. When two arrive at once, the line says the later one.
    /// </summary>
    public static IReadOnlyList<string> MilestoneOrder { get; } =
        ["words_1000", "words_10000", "words_50000", "words_100000", "streak_7", "streak_30", "streak_100"];

    /// <summary>The ids of this model's setting commands.</summary>
    public static IReadOnlySet<string> SettingIds { get; } = new HashSet<string>(StringComparer.Ordinal)
    {
        ShellSetting.StatsCelebrate.CommandId(),
        ShellSetting.StatsTypingWpm.CommandId(),
        ShellSetting.StatsRestDays.CommandId(),
        ShellSetting.StatsStreak.CommandId(),
        ShellSetting.StatsShareHeatmap.CommandId(),
        ShellSetting.StatsReviewDismissed.CommandId(),
    };

    /// <summary>A change to the streak's pause.</summary>
    public enum StreakChange
    {
        Pausing,
        Resuming,
    }

    /// <summary>Settings > Stats' words (the Mac's StatsSettingsSection, in Windows terms).</summary>
    public const string CelebrateTitle = "Celebrate milestones";
    public const string CelebrateDetail = "A quiet glow on the orb and one line, once for each milestone: 1,000 to 100,000 words, and streaks of 7, 30 and 100 active days. With Always still or Windows' animation effects off, only the line. A personal best gets a short note in the Drop.";
    public const string TypingTitle = "Typing speed";
    public const string TypingDetail = "Time saved is typing the same words at this speed, less the time spent speaking.";
    public const string SettingsFailedText = "Couldn't read or save a Stats setting. It may not be what it shows.";
    public const string WhereText = "Counted on this PC from your library. Nothing is sent, and nothing is compared with anyone.";
    public const string StreakTitle = "Show the streak";
    public const string StreakDetail = "Off, no streak shows on Stats or the share card, and its milestones aren't celebrated. It's still counted, so turning it back on loses nothing.";
    public const string RestDaysTitle = "Rest days";
    public const string RestDaysDetail = "Days you take off. They neither count toward a streak nor break it, even if you dictate on one.";
    public const string PauseTitle = "Pause the streak";
    public const string PauseDetail = "For a holiday or time off: days without a dictation don't count against the streak, for up to 90 days.";
    public const string LastRestDayHelp = "At least one day counts toward the streak";
    public const string ReviewDismissFailedText = "Couldn't dismiss it. It stays until you try again.";

    /// <summary>What the pause row says: running since a day, or what a pause does.</summary>
    public static string PauseCaption(string? pausedSince, CultureInfo? culture = null) =>
        pausedSince is null ? PauseDetail : $"Paused since {StatsFormat.ShortDate(pausedSince, culture)}. It ends by itself after 90 days, or when you resume.";

    /// <summary>What the pause row says before the numbers are in: counting, or that they couldn't be.</summary>
    public static string PauseUnknown(bool failed) => failed ? "Couldn't read whether the streak is paused." : "Counting…";

    /// <summary>What a failed pause or resume says.</summary>
    public static string StreakFailure(StreakChange change) =>
        change == StreakChange.Pausing ? "Couldn't pause the streak. Try again." : "Couldn't resume the streak. Try again.";

    /// <summary>The typing speed as Narrator reads it.</summary>
    public static string TypingSpoken(int wpm) => $"{wpm} words per minute";

    /// <summary>The prefix of stats.get's refs, and of milestones.check's.</summary>
    public const string StatsRefPrefix = "stats-";
    public const string MilestonesRefPrefix = "milestones-";
    public const string StreakRefPrefix = "streak-";

    private readonly Action<CoreCommand> send;
    private readonly IWakeScheduler wake;
    private readonly Func<TimeZoneInfo> zone;
    private readonly Func<DateTimeOffset> now;
    private int sequence;
    private string? latestGet;
    private IDisposable? loadTimer;
    private IDisposable? streakTimer;
    /// <summary>The pause or resume not answered yet.</summary>
    private string? pendingStreak;
    private HashSet<int> restDays = [];

    /// <summary>
    /// The Stats screen shows, and its window is on screen: it then counts again when something is
    /// saved. Off screen it waits, and counts when it is back.
    /// </summary>
    private bool screenShown;
    private bool windowOnScreen = true;
    private int celebrationSerial;

    /// <summary>
    /// The celebration whose glow, and whose line read aloud, have played: each once, even if the
    /// window leaves the screen and comes back while the line still waits.
    /// </summary>
    private int? glowedSerial;
    private int? announcedSerial;

    /// <summary>
    /// Each setting's own writes not yet echoed by the core: an echo of an earlier write is not
    /// taken over a later one (quick clicks on the speed would step back).
    /// </summary>
    private readonly Dictionary<ShellSetting, int> unechoed = [];

    /// <param name="wake">The load's time limit (the view's clock).</param>
    /// <param name="zone">The user's time zone, asked for each question (null: this PC's, as it is then).</param>
    /// <param name="culture">Their culture: where weeks start, and how counts read (null: this PC's).</param>
    /// <param name="now">The clock (null: the system's).</param>
    public StatsModel(Action<CoreCommand> send, IWakeScheduler wake, Func<TimeZoneInfo>? zone = null, CultureInfo? culture = null, Func<DateTimeOffset>? now = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        ArgumentNullException.ThrowIfNull(wake);
        this.send = send;
        this.wake = wake;
        this.zone = zone ?? LocalZoneNow;
        Culture = culture ?? CultureInfo.CurrentCulture;
        this.now = now ?? (() => DateTimeOffset.UtcNow);
    }

    /// <summary>The culture the numbers and dates read in.</summary>
    public CultureInfo Culture { get; }

    /// <summary>The core's latest answer.</summary>
    public StatsCounted? Counted { get; private set; }

    public Load LoadState { get; private set; } = Load.Idle;

    /// <summary>The milestone to celebrate, until its note has been shown.</summary>
    public Celebration? Pending { get; private set; }

    /// <summary>Settings > Stats: milestones are celebrated (on unless turned off).</summary>
    public bool Celebrate { get; private set; } = true;

    /// <summary>Settings > Stats: the typing speed time saved is measured against.</summary>
    public int TypingWpm { get; private set; } = DefaultTypingWpm;

    /// <summary>A Stats setting could not be read or saved: Settings says so (it may not be what it shows).</summary>
    public bool SettingsFailed { get; private set; }

    /// <summary>Settings > Stats: the weekdays the streak rests on (ISO, 1 Monday to 7 Sunday).</summary>
    public IReadOnlySet<int> RestDays => restDays;

    /// <summary>Settings > Stats: the streak shows (on unless hidden).</summary>
    public bool StreakShown { get; private set; } = true;

    /// <summary>The share card may carry the heatmap (off unless turned on).</summary>
    public bool ShareHeatmap { get; private set; }

    /// <summary>A pause or a resume of the streak sent and not answered yet.</summary>
    public StreakChange? StreakChanging { get; private set; }

    /// <summary>The pause or resume that failed, until the next is sent.</summary>
    public StreakChange? StreakChangeFailed { get; private set; }

    /// <summary>The week whose review the user dismissed (its first day): hidden at once.</summary>
    public string? DismissedWeek { get; private set; }

    /// <summary>The week whose dismissal could not be saved: its review shows again, and says so.</summary>
    public string? ReviewDismissFailedWeek { get; private set; }

    /// <summary>The review showing is one whose dismissal could not be saved.</summary>
    public bool ReviewDismissFailed => ReviewDismissFailedWeek is not null && WeekReview?.Week == ReviewDismissFailedWeek;

    /// <summary>Last week's review, until the user dismisses it (the core then leaves it out).</summary>
    public WeekReview? WeekReview => Counted?.WeekReview is { } review && review.Week != DismissedWeek ? review : null;

    private bool Visible => screenShown && windowOnScreen;

    private string Ref(string prefix) => $"{prefix}{++sequence}";

    /// <summary>The calendar fields both questions carry.</summary>
    private (IReadOnlyList<UtcOffset> Offsets, int WeekStart) CalendarFields() =>
        (UtcOffsets(zone(), now()), IsoWeekStart(Culture));

    /// <summary>This PC's zone as it is now: .NET keeps the first it read until its cache is cleared.</summary>
    private static TimeZoneInfo LocalZoneNow()
    {
        TimeZoneInfo.ClearCachedData();
        return TimeZoneInfo.Local;
    }

    /// <summary>Counts again.</summary>
    public void Reload()
    {
        if (LoadState != Load.Loaded)
        {
            LoadState = Load.Loading;
        }
        var reference = Ref(StatsRefPrefix);
        latestGet = reference;
        var (offsets, weekStart) = CalendarFields();
        send(new CoreCommand.StatsGet(offsets, weekStart, reference));
        // An answer that never comes is said, not spun for ever: one wait per question.
        loadTimer?.Dispose();
        loadTimer = wake.After(LoadLimit, () => LoadTimedOut(reference));
        Changed();
    }

    /// <summary>Asks what is newly reached.</summary>
    public void CheckMilestones()
    {
        var (offsets, weekStart) = CalendarFields();
        send(new CoreCommand.MilestonesCheck(offsets, weekStart, Ref(MilestonesRefPrefix)));
    }

    public void ScreenAppeared()
    {
        screenShown = true;
        Reload();
    }

    public void ScreenDisappeared() => screenShown = false;

    /// <summary>The window came on screen or left it. Back on screen, a showing Stats counts again.</summary>
    public void WindowPresence(bool onScreen)
    {
        var was = Visible;
        windowOnScreen = onScreen;
        if (Visible && !was)
        {
            Reload();
        }
    }

    /// <summary>
    /// The numbers did not come within the time limit: said, rather than a spinner for ever. Only
    /// while nothing is shown yet: a refresh that hangs leaves the last numbers up.
    /// </summary>
    public void LoadTimedOut(string reference)
    {
        if (reference != latestGet || LoadState != Load.Loading)
        {
            return;
        }
        LoadState = Load.Failed;
        Changed();
    }

    /// <summary>Settings shows the pause from the numbers: counted once when it first shows.</summary>
    public void SettingsAppeared()
    {
        if (Counted is null && LoadState != Load.Loading)
        {
            Reload();
        }
    }

    /// <summary>Dismisses last week's review for good: hidden now, kept by the core.</summary>
    public void DismissReview(WeekReview review)
    {
        ArgumentNullException.ThrowIfNull(review);
        DismissedWeek = review.Week;
        ReviewDismissFailedWeek = null;
        Write(ShellSetting.StatsReviewDismissed, review.Week);
        Changed();
    }

    /// <summary>Makes <paramref name="iso"/> a rest day or not. All seven never are: a streak needs a day to count.</summary>
    public void SetRestDay(int iso, bool on)
    {
        if (iso is < 1 or > 7)
        {
            return;
        }
        var days = new HashSet<int>(restDays);
        if (on)
        {
            days.Add(iso);
        }
        else
        {
            days.Remove(iso);
        }
        if (days.Count >= 7 || days.SetEquals(restDays))
        {
            return;
        }
        restDays = days;
        Write(ShellSetting.StatsRestDays, RestDaysValue(days));
        Changed();
    }

    public void SetStreakShown(bool shown)
    {
        StreakShown = shown;
        Write(ShellSetting.StatsStreak, shown ? "shown" : "hidden");
        Changed();
    }

    public void SetShareHeatmap(bool on)
    {
        ShareHeatmap = on;
        Write(ShellSetting.StatsShareHeatmap, on ? "on" : "off");
        Changed();
    }

    /// <summary>Pauses the streak from today: answered with the numbers.</summary>
    public void PauseStreak() => ChangeStreak(StreakChange.Pausing);

    /// <summary>Ends the running pause: answered with the numbers.</summary>
    public void ResumeStreak() => ChangeStreak(StreakChange.Resuming);

    private void ChangeStreak(StreakChange change)
    {
        if (StreakChanging is not null)
        {
            return;
        }
        var reference = Ref(StreakRefPrefix);
        StreakChanging = change;
        StreakChangeFailed = null;
        pendingStreak = reference;
        // Its answer is the newest count.
        latestGet = reference;
        var (offsets, weekStart) = CalendarFields();
        send(change == StreakChange.Pausing
            ? new CoreCommand.StreakPause(offsets, weekStart, reference)
            : new CoreCommand.StreakResume(offsets, weekStart, reference));
        // Never answered: said once the screen's time limit passes, and the button works again.
        streakTimer?.Dispose();
        streakTimer = wake.After(LoadLimit, () => StreakTimedOut(reference));
        Changed();
    }

    /// <summary>The pause or resume was not answered within the time limit: said, and the button works again.</summary>
    public void StreakTimedOut(string reference)
    {
        if (reference != pendingStreak)
        {
            return;
        }
        StreakFailed();
        // Its answer was to be the screen's numbers: none came.
        if (reference == latestGet && LoadState == Load.Loading)
        {
            LoadState = Load.Failed;
            loadTimer?.Dispose();
            loadTimer = null;
        }
        Changed();
    }

    private void StreakFailed()
    {
        StreakChangeFailed = StreakChanging;
        StreakChanging = null;
        pendingStreak = null;
        streakTimer?.Dispose();
        streakTimer = null;
    }

    /// <summary>stats.rest_days' value for <paramref name="days"/>: <c>none</c>, or ascending and comma-separated.</summary>
    public static string RestDaysValue(IEnumerable<int> days)
    {
        ArgumentNullException.ThrowIfNull(days);
        var sorted = days.Order().ToList();
        return sorted.Count == 0 ? "none" : string.Join(",", sorted.Select(d => d.ToString(CultureInfo.InvariantCulture)));
    }

    /// <summary>
    /// The rest days in stats.rest_days' value, read as the core reads it: anything it would not
    /// take (a day twice, out of order, all seven) is none.
    /// </summary>
    public static IReadOnlySet<int> ParseRestDays(string? value)
    {
        if (value is null or "none")
        {
            return new HashSet<int>();
        }
        var days = new List<int>();
        foreach (var part in value.Split(','))
        {
            if (part.Length != 1 || part[0] is < '1' or > '7' || part[0] - '0' <= (days.Count == 0 ? 0 : days[^1]))
            {
                return new HashSet<int>();
            }
            days.Add(part[0] - '0');
        }
        return days.Count < 7 ? days.ToHashSet() : new HashSet<int>();
    }

    public void SetCelebrate(bool on)
    {
        Celebrate = on;
        Write(ShellSetting.StatsCelebrate, on ? "on" : "off");
        Changed();
    }

    /// <summary>Sets the typing speed, within what the core takes.</summary>
    public void SetTypingWpm(int wpm)
    {
        TypingWpm = Math.Clamp(wpm, MinTypingWpm, MaxTypingWpm);
        Write(ShellSetting.StatsTypingWpm, TypingWpm.ToString(CultureInfo.InvariantCulture));
        Changed();
    }

    private void Write(ShellSetting key, string value)
    {
        unechoed[key] = unechoed.GetValueOrDefault(key) + 1;
        send(new CoreCommand.SettingSet(key, value));
    }

    /// <summary>
    /// Whether <paramref name="key"/>'s setting.value is an echo of one of this model's own writes
    /// with a later one still to come: it is then not taken. The last echo is (the core's word on it).
    /// </summary>
    private bool EarlierEcho(ShellSetting key)
    {
        if (unechoed.GetValueOrDefault(key) is not (> 0 and var pending))
        {
            return false;
        }
        unechoed[key] = pending - 1;
        return pending > 1;
    }

    /// <summary>The glow for <paramref name="serial"/> may play: true once per celebration.</summary>
    public bool BeginGlow(int serial)
    {
        if (glowedSerial == serial)
        {
            return false;
        }
        glowedSerial = serial;
        return true;
    }

    /// <summary>The line for <paramref name="serial"/> may be read aloud: true once per celebration.</summary>
    public bool BeginAnnouncement(int serial)
    {
        if (announcedSerial == serial)
        {
            return false;
        }
        announcedSerial = serial;
        return true;
    }

    /// <summary>The note has been shown for its time (or dismissed): it goes, unless a later one replaced it.</summary>
    public void DismissCelebration(int serial)
    {
        if (Pending?.Serial == serial)
        {
            Pending = null;
            Changed();
        }
    }

    /// <summary>
    /// Whether this model shows <paramref name="failed"/> itself. (A milestone check that failed
    /// celebrates nothing until the next one; no screen shows it, so it is logged.)
    /// </summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command switch
        {
            "stats.get" => failed.Id?.StartsWith(StatsRefPrefix, StringComparison.Ordinal) ?? false,
            "streak.pause" or "streak.resume" => failed.Id?.StartsWith(StreakRefPrefix, StringComparison.Ordinal) ?? false,
            "setting.get" or "setting.set" => failed.Id is string id && SettingIds.Contains(id),
            _ => false,
        };
    }

    public void Apply(InkEvent e)
    {
        if (Fold(e))
        {
            Changed();
        }
    }

    private bool Fold(InkEvent e)
    {
        switch (e)
        {
            case CoreReady:
                // A core that started again answers none of the old one's writes.
                unechoed.Clear();
                send(new CoreCommand.SettingGet(ShellSetting.StatsCelebrate));
                send(new CoreCommand.SettingGet(ShellSetting.StatsTypingWpm));
                send(new CoreCommand.SettingGet(ShellSetting.StatsRestDays));
                send(new CoreCommand.SettingGet(ShellSetting.StatsStreak));
                send(new CoreCommand.SettingGet(ShellSetting.StatsShareHeatmap));
                CheckMilestones();
                return false;
            case DictationInserted { Record: not null }:
            case MeetingFinished:
                SomethingSaved();
                return false;
            case ImportFinished:
                if (Visible)
                {
                    Reload();
                }
                return false;
            case StatsCounted answer when answer.Ref is not null && (answer.Ref == latestGet || answer.Ref == pendingStreak):
                if (answer.Ref == pendingStreak)
                {
                    pendingStreak = null;
                    StreakChanging = null;
                    streakTimer?.Dispose();
                    streakTimer = null;
                }
                if (answer.Ref != latestGet)
                {
                    return true;
                }
                // A pause answered after its time limit did change: it failed only as far as was known.
                if (answer.Ref.StartsWith(StreakRefPrefix, StringComparison.Ordinal))
                {
                    StreakChangeFailed = null;
                }
                Counted = answer;
                LoadState = Load.Loaded;
                loadTimer?.Dispose();
                loadTimer = null;
                return true;
            case MilestonesReached answer when answer.Ref?.StartsWith(MilestonesRefPrefix, StringComparison.Ordinal) ?? false:
                return Reached(answer);
            case SettingValue { Key: "stats.celebrate" } value:
                if (EarlierEcho(ShellSetting.StatsCelebrate))
                {
                    return false;
                }
                SettingsFailed = false;
                Celebrate = value.Value != "off";
                return true;
            case SettingValue { Key: "stats.typing_wpm" } value:
                if (EarlierEcho(ShellSetting.StatsTypingWpm))
                {
                    return false;
                }
                SettingsFailed = false;
                // Read as the core reads it: anything it would not take is the default.
                TypingWpm = int.TryParse(value.Value, NumberStyles.None, CultureInfo.InvariantCulture, out var wpm) && wpm is >= MinTypingWpm and <= MaxTypingWpm
                    ? wpm
                    : DefaultTypingWpm;
                // Time saved is counted against it; a hidden Stats counts when it shows.
                if (Visible && Counted is not null && Counted.TypingWpm != TypingWpm)
                {
                    Reload();
                }
                return true;
            case SettingValue { Key: "stats.rest_days" } value:
                if (EarlierEcho(ShellSetting.StatsRestDays))
                {
                    return false;
                }
                SettingsFailed = false;
                restDays = [.. ParseRestDays(value.Value)];
                // The streak is counted with them; a hidden Stats counts when it shows.
                if (Visible && Counted is not null && !restDays.SetEquals((Counted.Dictation.RestDays ?? []).Select(d => (int)d)))
                {
                    Reload();
                }
                return true;
            case SettingValue { Key: "stats.streak" } value:
                if (EarlierEcho(ShellSetting.StatsStreak))
                {
                    return false;
                }
                SettingsFailed = false;
                StreakShown = value.Value != "hidden";
                if (Visible && Counted is not null && (Counted.Dictation.StreakHidden ?? false) == StreakShown)
                {
                    Reload();
                }
                return true;
            case SettingValue { Key: "stats.share_heatmap" } value:
                if (EarlierEcho(ShellSetting.StatsShareHeatmap))
                {
                    return false;
                }
                SettingsFailed = false;
                ShareHeatmap = value.Value == "on";
                return true;
            case SettingValue { Key: "stats.review_dismissed" }:
                _ = EarlierEcho(ShellSetting.StatsReviewDismissed);
                return false;
            case CommandFailed { Command: "stats.get" } failed when failed.Id is not null && failed.Id == latestGet:
                LoadState = Load.Failed;
                loadTimer?.Dispose();
                loadTimer = null;
                return true;
            case CommandFailed { Command: "streak.pause" or "streak.resume" } failed when failed.Id is not null && failed.Id == pendingStreak:
                StreakFailed();
                // The pause's answer was to be the screen's numbers: none comes.
                if (failed.Id == latestGet && LoadState == Load.Loading)
                {
                    LoadState = Load.Failed;
                    loadTimer?.Dispose();
                    loadTimer = null;
                }
                return true;
            case CommandFailed failed when failed.Id == ShellSetting.StatsReviewDismissed.CommandId():
                if (failed.Command == "setting.set")
                {
                    _ = EarlierEcho(ShellSetting.StatsReviewDismissed);
                    // Not kept: the review shows again, and says so.
                    ReviewDismissFailedWeek = DismissedWeek;
                    DismissedWeek = null;
                }
                return true;
            case CommandFailed failed when failed.Id is string id && SettingIds.Contains(id):
                if (failed.Command == "setting.set" && SettingKey(id) is ShellSetting key)
                {
                    _ = EarlierEcho(key);
                }
                SettingsFailed = true;
                return true;
            default:
                return false;
        }
    }

    /// <summary>
    /// Every check's answer: each milestone is reported once ever. Usually none; several only
    /// after a long gap, or from checks that overlapped. One line says the biggest.
    /// </summary>
    private bool Reached(MilestonesReached answer)
    {
        int Rank(string id)
        {
            for (var i = 0; i < MilestoneOrder.Count; i++)
            {
                if (MilestoneOrder[i] == id)
                {
                    return i;
                }
            }
            return -1;
        }
        MilestoneRow? biggest = null;
        foreach (var m in answer.Milestones)
        {
            if (biggest is null || Rank(m.Id) > Rank(biggest.Id))
            {
                biggest = m;
            }
        }
        if (biggest is null || (Pending is not null && Rank(Pending.Id) >= Rank(biggest.Id)))
        {
            return false;
        }
        Pending = new Celebration(++celebrationSerial, biggest.Id, StatsFormat.MilestoneNote(biggest.Kind, biggest.Threshold, Culture, biggest.Name));
        return true;
    }

    private static ShellSetting? SettingKey(string id) =>
        id == ShellSetting.StatsCelebrate.CommandId() ? ShellSetting.StatsCelebrate
        : id == ShellSetting.StatsTypingWpm.CommandId() ? ShellSetting.StatsTypingWpm
        : id == ShellSetting.StatsRestDays.CommandId() ? ShellSetting.StatsRestDays
        : id == ShellSetting.StatsStreak.CommandId() ? ShellSetting.StatsStreak
        : id == ShellSetting.StatsShareHeatmap.CommandId() ? ShellSetting.StatsShareHeatmap
        : null;

    private void SomethingSaved()
    {
        CheckMilestones();
        if (Visible)
        {
            Reload();
        }
    }

    // The user's calendar

    /// <summary>
    /// <paramref name="zone"/>'s UTC offset over the last ten years: the offset then, and each
    /// change since, oldest first (what stats.get's utc_offsets takes). Every change, not only
    /// daylight saving's: a zone that moved its standard time (as North Korea's did in 2018) is
    /// sampled weekly, and each change found is narrowed to the second.
    /// </summary>
    public static IReadOnlyList<UtcOffset> UtcOffsets(TimeZoneInfo zone, DateTimeOffset now)
    {
        ArgumentNullException.ThrowIfNull(zone);
        var start = now.AddYears(-OffsetYears);
        int Minutes(DateTimeOffset at) => (int)Math.Round(zone.GetUtcOffset(at).TotalMinutes);
        var offsets = new List<UtcOffset> { new(start.ToUnixTimeMilliseconds(), Minutes(start)) };
        var week = TimeSpan.FromDays(7);
        var from = start;
        // Two changes a year at most in any zone, and one week cannot hold two: the cap only
        // guards a zone database gone wrong.
        while (from < now && offsets.Count < 300)
        {
            var to = from + week < now ? from + week : now;
            if (Minutes(to) != Minutes(from))
            {
                // The first second with the new offset.
                var (before, after) = (from, to);
                while (after - before > TimeSpan.FromSeconds(1))
                {
                    var middle = before + (after - before) / 2;
                    if (Minutes(middle) == Minutes(from))
                    {
                        before = middle;
                    }
                    else
                    {
                        after = middle;
                    }
                }
                // Zones change on a whole second, the one after `before`.
                var change = DateTimeOffset.FromUnixTimeSeconds(before.ToUnixTimeSeconds() + 1);
                if (Minutes(to) != offsets[^1].Minutes)
                {
                    offsets.Add(new UtcOffset((change < after ? change : after).ToUnixTimeMilliseconds(), Minutes(to)));
                }
            }
            from = to;
        }
        return offsets;
    }

    /// <summary>The ISO weekday (1 Monday to 7 Sunday) <paramref name="culture"/> starts weeks on.</summary>
    public static int IsoWeekStart(CultureInfo culture)
    {
        ArgumentNullException.ThrowIfNull(culture);
        var first = culture.DateTimeFormat.FirstDayOfWeek;
        return first == DayOfWeek.Sunday ? 7 : (int)first;
    }
}
