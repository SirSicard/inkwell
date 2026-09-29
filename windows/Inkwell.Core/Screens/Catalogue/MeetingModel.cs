// Meetings as the user drives them, as the Mac's MeetingModel: record now, stop, and the settings
// that shape them: listening for calls, the headset mic, and how long the library keeps records.
//
// Recording starts only when the user asks (Today's "Record now", the menu, and the Drop's
// "Record this call"). Detection only offers. A meeting's title comes from the calendar when a
// call is on it now, read without prompting (Windows: NoCalendar until packaging, so none);
// otherwise the summary's headline names it later.
//
// Not here yet: the consent Drop's answers ("Record this call" for an app the core offered, "Not
// this one": meeting.detected -> the Drop). The Drop step adds them, with their places and
// meeting.dismiss's failure, alongside the two origins below.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>How long the library keeps records (<c>retention.days</c>).</summary>
public enum Retention
{
    Forever,
    Week,
    Month,
    Quarter,
    Year,
}

public static class Retentions
{
    /// <summary>Every choice, in the picker's order.</summary>
    public static IReadOnlyList<Retention> All { get; } = [Retention.Forever, Retention.Week, Retention.Month, Retention.Quarter, Retention.Year];

    /// <summary>Its value in the core's store (the core's whitelist).</summary>
    public static string Value(this Retention retention) => retention switch
    {
        Retention.Forever => "forever",
        Retention.Week => "7",
        Retention.Month => "30",
        Retention.Quarter => "90",
        Retention.Year => "365",
        _ => throw new ArgumentOutOfRangeException(nameof(retention)),
    };

    public static string Title(this Retention retention) => retention switch
    {
        Retention.Forever => "Forever",
        Retention.Week => "A week",
        Retention.Month => "30 days",
        Retention.Quarter => "90 days",
        Retention.Year => "A year",
        _ => throw new ArgumentOutOfRangeException(nameof(retention)),
    };

    /// <summary>The choice a stored value names; null for a value that is none of them.</summary>
    public static Retention? Parse(string? value) => All.Where(r => r.Value() == value).Select(r => (Retention?)r).FirstOrDefault();
}

/// <summary>Where a meeting command was asked for, so its failure shows there and nowhere else.</summary>
public enum MeetingOrigin
{
    /// <summary>Record now (Today, or Live with no meeting).</summary>
    RecordNow,
    /// <summary>Stop, in Live.</summary>
    Stop,
}

/// <summary>The places that show a meeting command's failure.</summary>
public enum MeetingPlace
{
    /// <summary>Record now's button (Today's foot, Live with no meeting).</summary>
    RecordNow,
    /// <summary>Live's header, beside Stop.</summary>
    LiveStop,
}

/// <summary>The last meeting command's failure: where it was asked, and the core's words (they never quote the user).</summary>
public sealed record MeetingFailure(MeetingOrigin Origin, string Message);

public sealed class MeetingModel(
    Action<CoreCommand> send, ICallTitles? titles = null, Func<DateTimeOffset>? now = null, ScreenLog? log = null) : ObservableModel
{
    public const string SettingsFailedText = "Couldn't read or save a meeting setting. It may not be what it shows.";

    /// <summary>The ids of this model's setting commands (CoreCommand gives each setting command one).</summary>
    public static IReadOnlySet<string> SettingIds { get; } = new HashSet<string>(StringComparer.Ordinal)
    {
        ShellSetting.MeetingsDetect.CommandId(),
        ShellSetting.MeetingsHeadsetMic.CommandId(),
        ShellSetting.RetentionDays.CommandId(),
    };

    private readonly ICallTitles titles = titles ?? NoCalendar.Instance;
    private readonly Func<DateTimeOffset> now = now ?? (() => DateTimeOffset.Now);
    private readonly ScreenLog log = log ?? ScreenLog.System;
    /// <summary>Where the start in flight was asked for (a start's command id is the same from every place).</summary>
    private MeetingOrigin starting = MeetingOrigin.RecordNow;

    public MeetingFailure? Failure { get; private set; }

    /// <summary>Whether the core listens for calls (the user's setting; on unless turned off).</summary>
    public bool Detect { get; private set; } = true;

    /// <summary>With Bluetooth output, record the headset's own mic.</summary>
    public bool HeadsetMic { get; private set; }

    /// <summary>How long the library keeps records; null until the store answers (the picker is disabled then).</summary>
    public Retention? Retention { get; private set; }

    /// <summary>A setting could not be read or saved: its control says so.</summary>
    public bool SettingsFailed { get; private set; }

    /// <summary>What <paramref name="place"/> says about the last failure, or null when it was not asked there.</summary>
    public string? FailureOn(MeetingPlace place) => (place, Failure?.Origin) switch
    {
        (MeetingPlace.RecordNow, MeetingOrigin.RecordNow) => $"Couldn't start recording: {Failure!.Message}",
        (MeetingPlace.LiveStop, MeetingOrigin.Stop) => $"Couldn't stop: {Failure!.Message}",
        _ => null,
    };

    public void Load()
    {
        send(new CoreCommand.SettingGet(ShellSetting.MeetingsDetect));
        send(new CoreCommand.SettingGet(ShellSetting.MeetingsHeadsetMic));
        send(new CoreCommand.SettingGet(ShellSetting.RetentionDays));
    }

    /// <summary>Records now, the whole of what this PC plays as the far end.</summary>
    public void RecordNow()
    {
        Failure = null;
        starting = MeetingOrigin.RecordNow;
        Changed();
        send(new CoreCommand.MeetingStart(null, titles.TitleNow(now())));
    }

    public void Stop()
    {
        Failure = null;
        Changed();
        send(new CoreCommand.MeetingStop());
    }

    public void SetDetect(bool on)
    {
        Detect = on;
        Changed();
        send(new CoreCommand.SettingSet(ShellSetting.MeetingsDetect, on ? "on" : "off"));
    }

    public void SetHeadsetMic(bool on)
    {
        HeadsetMic = on;
        Changed();
        send(new CoreCommand.SettingSet(ShellSetting.MeetingsHeadsetMic, on ? "on" : "off"));
    }

    public void SetRetention(Retention value)
    {
        Retention = value;
        Changed();
        send(new CoreCommand.SettingSet(ShellSetting.RetentionDays, value.Value()));
    }

    /// <summary>
    /// Whether this model shows the failure: meeting.start and meeting.stop, and its settings'
    /// setting.get and setting.set (matched by id).
    /// </summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command switch
        {
            "meeting.start" or "meeting.stop" => true,
            "setting.get" or "setting.set" => failed.Id is string id && SettingIds.Contains(id),
            _ => false,
        };
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case SettingValue value when value.Key == ShellSetting.MeetingsDetect.Key():
                Detect = value.Value != "off";
                Changed();
                break;
            case SettingValue value when value.Key == ShellSetting.MeetingsHeadsetMic.Key():
                HeadsetMic = value.Value == "on";
                Changed();
                break;
            case SettingValue value when value.Key == ShellSetting.RetentionDays.Key():
                Retention = Retentions.Parse(value.Value) ?? Screens.Retention.Forever;
                Changed();
                break;
            case MeetingStarted:
                Failure = null;
                Changed();
                break;
            case CommandFailed failed when failed.Command is "meeting.start" or "meeting.stop":
                var origin = failed.Command == "meeting.stop" ? MeetingOrigin.Stop : starting;
                Failure = new MeetingFailure(origin, failed.Message);
                Changed();
                // The core logged why; this says the shell showed it (the controller does not log a
                // failure a screen shows). The command's name only, never the core's words.
                log.Write($"command.failed for a {failed.Command} command; shown where it was asked");
                break;
            case CommandFailed failed when failed.Id is string id && SettingIds.Contains(id):
                // Not known, or not saved: the control shows it, never a guessed value.
                SettingsFailed = true;
                Changed();
                break;
            default:
                break;
        }
    }
}
