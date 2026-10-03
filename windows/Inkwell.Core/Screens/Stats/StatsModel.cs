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
// The days are the user's current zone's: a record made while travelling is placed by the zone
// the PC is in now, as the store keeps no zone per record.
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
    };

    /// <summary>Settings > Stats' words (the Mac's StatsSettingsSection, in Windows terms).</summary>
    public const string CelebrateTitle = "Celebrate milestones";
    public const string CelebrateDetail = "A quiet glow on the orb and one line, once for each milestone: 1,000 to 100,000 words, and streaks of 7, 30 and 100 active days. With Always still or Windows' animation effects off, only the line.";
    public const string TypingTitle = "Typing speed";
    public const string TypingDetail = "Time saved is typing the same words at this speed, less the time spent speaking.";
    public const string SettingsFailedText = "Couldn't read or save a Stats setting. It may not be what it shows.";
    public const string WhereText = "Counted on this PC from your library. Nothing is sent, and nothing is compared with anyone.";

    /// <summary>The typing speed as Narrator reads it.</summary>
    public static string TypingSpoken(int wpm) => $"{wpm} words per minute";

    /// <summary>The prefix of stats.get's refs, and of milestones.check's.</summary>
    public const string StatsRefPrefix = "stats-";
    public const string MilestonesRefPrefix = "milestones-";

    private readonly Action<CoreCommand> send;
    private readonly IWakeScheduler wake;
    private readonly TimeZoneInfo zone;
    private readonly Func<DateTimeOffset> now;
    private int sequence;
    private string? latestGet;
    private IDisposable? loadTimer;

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
    /// <param name="zone">The user's time zone (null: this PC's).</param>
    /// <param name="culture">Their culture: where weeks start, and how counts read (null: this PC's).</param>
    /// <param name="now">The clock (null: the system's).</param>
    public StatsModel(Action<CoreCommand> send, IWakeScheduler wake, TimeZoneInfo? zone = null, CultureInfo? culture = null, Func<DateTimeOffset>? now = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        ArgumentNullException.ThrowIfNull(wake);
        this.send = send;
        this.wake = wake;
        this.zone = zone ?? TimeZoneInfo.Local;
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

    private bool Visible => screenShown && windowOnScreen;

    private string Ref(string prefix) => $"{prefix}{++sequence}";

    /// <summary>The calendar fields both questions carry.</summary>
    private (IReadOnlyList<UtcOffset> Offsets, int WeekStart) CalendarFields() =>
        (UtcOffsets(zone, now()), IsoWeekStart(Culture));

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
            case StatsCounted answer when answer.Ref is not null && answer.Ref == latestGet:
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
            case CommandFailed { Command: "stats.get" } failed when failed.Id is not null && failed.Id == latestGet:
                LoadState = Load.Failed;
                loadTimer?.Dispose();
                loadTimer = null;
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
        Pending = new Celebration(++celebrationSerial, biggest.Id, StatsFormat.MilestoneNote(biggest.Kind, biggest.Threshold, Culture));
        return true;
    }

    private static ShellSetting? SettingKey(string id) =>
        id == ShellSetting.StatsCelebrate.CommandId() ? ShellSetting.StatsCelebrate
        : id == ShellSetting.StatsTypingWpm.CommandId() ? ShellSetting.StatsTypingWpm
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
