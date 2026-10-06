// Meetings as the user drives them, as the Mac's MeetingModel: record now, record the call the
// Drop offers or say not this one, stop, stop and delete, and the settings that shape them: the
// headset mic, and how long the library keeps records (the call policies, which replaced listening
// for calls, are CallPolicyModel's).
//
// Recording starts when the user asks (Today's "Record now", the menu, and the Drop's "Record
// this call"), or for an app the user chose Always for: the core starts that one itself, and the
// Drop shows it with Stop, and Stop and delete for its first minute (one wake ends it). A
// meeting's title comes from the calendar when a call is on it now, read without prompting
// (Windows: NoCalendar until packaging, so none); otherwise the summary's headline names it later.
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
    /// <summary>"Record this call", on the Drop.</summary>
    Offer,
    /// <summary>"Not this one", on the Drop.</summary>
    Dismiss,
    /// <summary>Stop, in Live.</summary>
    Stop,
    /// <summary>Stop, on the Drop of a call its app's Always recorded.</summary>
    DropStop,
    /// <summary>"Stop and delete", on that Drop.</summary>
    Discard,
}

/// <summary>The places that show a meeting command's failure.</summary>
public enum MeetingPlace
{
    /// <summary>Record now's button (Today's foot, Live with no meeting).</summary>
    RecordNow,
    /// <summary>The Drop, whose buttons answer an offer.</summary>
    Drop,
    /// <summary>Live's header, beside Stop.</summary>
    LiveStop,
}

/// <summary>The last meeting command's failure: where it was asked, and the core's words (they never quote the user).</summary>
public sealed record MeetingFailure(MeetingOrigin Origin, string Message);

public sealed class MeetingModel(
    Action<CoreCommand> send, ICallTitles? titles = null, Func<DateTimeOffset>? now = null, ScreenLog? log = null,
    IWakeScheduler? wake = null) : ObservableModel
{
    public const string SettingsFailedText = "Couldn't read or save a meeting setting. It may not be what it shows.";
    public const string DeleteWindowOverText = "The first minute is over. Stop it, then delete it in the Library.";

    /// <summary>The ids of this model's setting commands (CoreCommand gives each setting command one).</summary>
    public static IReadOnlySet<string> SettingIds { get; } = new HashSet<string>(StringComparer.Ordinal)
    {
        ShellSetting.MeetingsHeadsetMic.CommandId(),
        ShellSetting.RetentionDays.CommandId(),
    };

    private readonly ICallTitles titles = titles ?? NoCalendar.Instance;
    private readonly Func<DateTimeOffset> now = now ?? (() => DateTimeOffset.Now);
    private readonly ScreenLog log = log ?? ScreenLog.System;
    private readonly IWakeScheduler? wake = wake;
    /// <summary>Where the start in flight was asked for (a start's command id is the same from every place).</summary>
    private MeetingOrigin starting = MeetingOrigin.RecordNow;
    /// <summary>Where the stop in flight was asked for.</summary>
    private MeetingOrigin stopping = MeetingOrigin.Stop;
    /// <summary>The one wake that ends Stop and delete's minute.</summary>
    private IDisposable? deleteDeadline;
    /// <summary>The meeting live now, as meeting.started named it: only its end clears its Drop failures.</summary>
    private string? current;
    /// <summary>
    /// The app the core offers now, as the store keeps it (meeting.detected, until its
    /// detection_ended, detection stopping, or a meeting starting): a Drop answer's failure belongs
    /// to this offer, and goes with it.
    /// </summary>
    private string? offered;

    public MeetingFailure? Failure { get; private set; }

    /// <summary>
    /// The meeting Stop and delete can still delete: one its app's Always started, until its
    /// delete_until_unix_ms. Cleared at that moment by one scheduled wake, never by polling.
    /// </summary>
    public string? Deletable { get; private set; }

    /// <summary>The meeting being stopped and deleted, until the core says it is gone (or refuses). Live reads it: its notes are never saved.</summary>
    public string? Discarding { get; private set; }

    /// <summary>With Bluetooth output, record the headset's own mic.</summary>
    public bool HeadsetMic { get; private set; }

    /// <summary>How long the library keeps records; null until the store answers (the picker is disabled then).</summary>
    public Retention? Retention { get; private set; }

    /// <summary>A setting could not be read or saved: its control says so.</summary>
    public bool SettingsFailed { get; private set; }

    /// <summary>What <paramref name="place"/> says about the last failure, or null when it was not asked there.</summary>
    public string? FailureOn(MeetingPlace place) => (place, Failure?.Origin) switch
    {
        (MeetingPlace.RecordNow, MeetingOrigin.RecordNow) or (MeetingPlace.Drop, MeetingOrigin.Offer) =>
            $"Couldn't start recording: {Failure!.Message}",
        (MeetingPlace.Drop, MeetingOrigin.Dismiss) => $"Couldn't dismiss the offer: {Failure!.Message}",
        (MeetingPlace.LiveStop, MeetingOrigin.Stop) or (MeetingPlace.Drop, MeetingOrigin.DropStop) => $"Couldn't stop: {Failure!.Message}",
        (MeetingPlace.Drop, MeetingOrigin.Discard) => Failure!.Message,
        _ => null,
    };

    /// <summary>Whether Stop and delete is offered for <paramref name="record"/> now.</summary>
    public bool CanDiscard(string? record) => record is not null && Deletable == record && Discarding is null;

    public void Load()
    {
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

    /// <summary>Records the call the core offered (<paramref name="app"/>, as the offer names it).</summary>
    public void Record(string app)
    {
        ArgumentNullException.ThrowIfNull(app);
        Failure = null;
        starting = MeetingOrigin.Offer;
        Changed();
        send(new CoreCommand.MeetingStart(app, titles.TitleNow(now())));
    }

    /// <summary>"Not this one": the offer goes, and that app is not offered again until it lets go of the mic.</summary>
    public void Dismiss(string app)
    {
        ArgumentNullException.ThrowIfNull(app);
        Failure = null;
        Changed();
        send(new CoreCommand.MeetingDismiss(app));
    }

    /// <summary>A meeting button on the Drop (Always for and Never for are the call policies').</summary>
    public void Perform(DropAction action)
    {
        ArgumentNullException.ThrowIfNull(action);
        switch (action)
        {
            case DropAction.Record record:
                Record(record.App);
                break;
            case DropAction.Dismiss dismiss:
                Dismiss(dismiss.App);
                break;
            case DropAction.StopRecording:
                Stop(MeetingOrigin.DropStop);
                break;
            case DropAction.StopAndDelete:
                Discard();
                break;
            default:
                throw new ArgumentOutOfRangeException(nameof(action));
        }
    }

    public void Stop() => Stop(MeetingOrigin.Stop);

    private void Stop(MeetingOrigin origin)
    {
        Failure = null;
        stopping = origin;
        Changed();
        send(new CoreCommand.MeetingStop());
    }

    /// <summary>"Stop and delete": only while the meeting's first minute lasts.</summary>
    public void Discard()
    {
        if (Deletable is not string record)
        {
            return;
        }
        Failure = null;
        Discarding = record;
        Changed();
        send(new CoreCommand.MeetingDiscard());
    }

    /// <summary>Words for a Stop and delete the core refused: past the minute, only Stop is left.</summary>
    public static string DiscardFailure(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Code == FailureCode.DeleteWindowOver ? DeleteWindowOverText : $"Couldn't delete it: {failed.Message}";
    }

    /// <summary>
    /// A meeting started: Stop and delete is offered until its deadline when its app's Always
    /// started it. One scheduled wake ends the offer (architecture rule 9: nothing polls). Only a
    /// start the policy made: one the user made has Live's Stop, and the library's delete afterwards.
    /// </summary>
    private void Started(MeetingStarted started)
    {
        EndDeleteWindow();
        Discarding = null;
        if (started.Auto != true || started.DeleteUntilUnixMs is not long until || wake is null)
        {
            return;
        }
        var left = DateTimeOffset.FromUnixTimeMilliseconds(until) - now();
        if (left <= TimeSpan.Zero)
        {
            return;
        }
        var record = started.Record;
        Deletable = record;
        IDisposable? mine = null;
        mine = wake.After(left, () =>
        {
            // Only this meeting's wake: a later start has its own.
            if (!ReferenceEquals(deleteDeadline, mine) || Deletable != record)
            {
                return;
            }
            deleteDeadline = null;
            Deletable = null;
            mine?.Dispose();
            Changed();
        });
        deleteDeadline = mine;
    }

    private void EndDeleteWindow()
    {
        deleteDeadline?.Dispose();
        deleteDeadline = null;
        Deletable = null;
    }

    /// <summary>
    /// The meeting <paramref name="record"/> ended, one way or another: its Drop buttons' failures
    /// go with it (an earlier meeting's final pass ending never takes the live one's).
    /// </summary>
    private void Ended(string record)
    {
        if (Deletable == record)
        {
            EndDeleteWindow();
        }
        if (Discarding == record)
        {
            Discarding = null;
        }
        if (current == record)
        {
            current = null;
            ClearMeetingFailure(stop: true);
        }
        Changed();
    }

    /// <summary>
    /// A Stop and delete failure said on the Drop, and with <paramref name="stop"/> a Stop's: Stop's
    /// stays while its meeting's Drop shows; Stop and delete's only until the transcript moves on (it
    /// said why the button went).
    /// </summary>
    private void ClearMeetingFailure(bool stop)
    {
        if (Failure?.Origin == MeetingOrigin.Discard || (stop && Failure?.Origin == MeetingOrigin.DropStop))
        {
            Failure = null;
            Changed();
        }
    }

    /// <summary>
    /// Finishes the meetings a crash interrupted (their final pass runs over what is on disk). Sent
    /// once the core is ready: this shell registers no engines of its own to wait for.
    /// </summary>
    public void Recover() => send(new CoreCommand.MeetingsRecover());

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
    /// Whether this model shows the failure: meeting.start, meeting.stop, meeting.dismiss and
    /// meeting.discard, and its settings' setting.get and setting.set (matched by id).
    /// </summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command switch
        {
            "meeting.start" or "meeting.stop" or "meeting.dismiss" or "meeting.discard" => true,
            "setting.get" or "setting.set" => failed.Id is string id && SettingIds.Contains(id),
            _ => false,
        };
    }

    /// <summary>A Drop answer's failure goes with its offer; Record now's and Stop's stay where they were asked.</summary>
    private void EndOffersFailure()
    {
        if (Failure?.Origin is MeetingOrigin.Offer or MeetingOrigin.Dismiss)
        {
            Failure = null;
            Changed();
        }
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case SettingValue value when value.Key == ShellSetting.MeetingsHeadsetMic.Key():
                HeadsetMic = value.Value == "on";
                Changed();
                break;
            case SettingValue value when value.Key == ShellSetting.RetentionDays.Key():
                Retention = Retentions.Parse(value.Value) ?? Screens.Retention.Forever;
                Changed();
                break;
            case MeetingStarted started:
                offered = null;
                Failure = null;
                current = started.Record;
                Started(started);
                Changed();
                break;
            case MeetingStopped stopped when Deletable == stopped.Record:
                // Stopped: Stop and delete is no longer offered (a discard in flight goes on).
                EndDeleteWindow();
                Changed();
                break;
            case MeetingFinal final when final.Record == current:
                ClearMeetingFailure(stop: false);
                break;
            case MeetingFinished finished:
                Ended(finished.Record);
                break;
            case MeetingDiscarded discarded:
                Ended(discarded.Record);
                break;
            case Events.MeetingFailed { Record: string failedRecord }:
                Ended(failedRecord);
                break;
            case Events.MeetingWorkerFailed workerFailed:
                Ended(workerFailed.Record);
                break;
            case CoreStopped:
                EndDeleteWindow();
                Discarding = null;
                current = null;
                Changed();
                break;
            case MeetingDetected detected:
                // A new offer is a new question: an earlier answer's failure is not its.
                offered = detected.App;
                EndOffersFailure();
                ClearMeetingFailure(stop: true);
                break;
            case MeetingDetectionEnded ended when ended.App == offered:
                offered = null;
                EndOffersFailure();
                break;
            case MeetingDetection { Listening: false }:
                offered = null;
                EndOffersFailure();
                break;
            case CommandFailed failed when failed.Command == "meeting.discard":
                // The command names no meeting: a refusal is this shell's only while it waits for
                // one (a meeting that ended meanwhile took its buttons with it).
                if (Discarding is null)
                {
                    log.Write("command.failed for a meeting.discard command; its meeting had ended, so nothing shows it");
                    break;
                }
                Discarding = null;
                // Past the minute only Stop is left; another refusal (a store that failed) may pass,
                // so the button stays for its minute to be pressed again.
                if (failed.Code == FailureCode.DeleteWindowOver)
                {
                    EndDeleteWindow();
                }
                Failure = new MeetingFailure(MeetingOrigin.Discard, DiscardFailure(failed));
                Changed();
                log.Write("command.failed for a meeting.discard command; shown where it was asked");
                break;
            case CommandFailed failed when failed.Command is "meeting.start" or "meeting.stop" or "meeting.dismiss":
                var origin = failed.Command switch
                {
                    "meeting.stop" => stopping,
                    "meeting.dismiss" => MeetingOrigin.Dismiss,
                    _ => starting,
                };
                if ((origin is MeetingOrigin.Offer or MeetingOrigin.Dismiss) && offered is null)
                {
                    // The offer it answered has gone (a second click after the first started the
                    // meeting, or the app let go of the mic): nothing shows it now, and a later
                    // offer must not. The core logged why.
                    log.Write($"command.failed for a {failed.Command} command; its offer had gone, so nothing shows it");
                    break;
                }
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
