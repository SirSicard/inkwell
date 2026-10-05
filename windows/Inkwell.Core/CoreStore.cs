// What the shell knows about the core, built only from the core's events, as the Mac's CoreStore.
// The screens read it and never ask the core for state: the core pushes, the shell renders
// (architecture rule 1).
//
// The controller feeds it batches on the UI thread; Apply folds a batch in one pass and signals one
// change per batch however many events it held. Event payloads can carry the user's words
// (partials, finals): the store holds them for the screens and never logs them.
using System.Collections.Immutable;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;

namespace Inkwell.Core;

/// <summary>Where a dictation is.</summary>
public enum DictationPhase
{
    Idle,
    /// <summary>The key is held and the mic is recorded.</summary>
    Listening,
    /// <summary>The key was released; the take is being transcribed and inserted.</summary>
    Transcribing,
}

/// <summary>The take being held or processed.</summary>
/// <param name="Partial">What the live engine hears so far, while the key is held: shown, never logged.</param>
public sealed record LiveDictation(long Take, bool Edit, string? Mode, string? App, string? Partial = null);

/// <summary>How the last dictation ended.</summary>
public abstract record DictationOutcome
{
    private DictationOutcome() { }

    public sealed record Inserted(InsertOutcome Outcome) : DictationOutcome;

    public sealed record Discarded(Discard Reason) : DictationOutcome;

    public sealed record Failed(FailedStage Stage) : DictationOutcome;
}

/// <summary>The live ledger's size (counts and bytes only, never text).</summary>
/// <param name="Seen">Finals received.</param>
/// <param name="Dropped">Finals let go of (older than the window).</param>
/// <param name="Bytes">UTF-8 bytes of the finals held now.</param>
/// <param name="PeakBytes">The most bytes held at once.</param>
public readonly record struct LedgerStats(int Seen, int Dropped, int Bytes, int PeakBytes);

/// <summary>The meeting being recorded, or finishing (its final pass runs after meeting.stopped).</summary>
public sealed record LiveMeeting(string Record)
{
    /// <summary>The most finals kept in memory: an hour's meeting has about 600 to 1,000.</summary>
    public const int FinalsKept = 500;

    /// <summary>Its title, when the core knew one at the start.</summary>
    public string? Title { get; init; }
    /// <summary>The app it records, by id, and that app's name, when it was started for one.</summary>
    public string? App { get; init; }
    public string? AppName { get; init; }
    /// <summary>The microphone it records, and why that one.</summary>
    public string? MicName { get; init; }
    public MicReason? MicReason { get; init; }
    /// <summary>What it records as the other side.</summary>
    public FarEnd? FarEnd { get; init; }
    /// <summary>It records everything the PC plays instead of the app alone (meeting.far_end_fallback).</summary>
    public bool FarEndFallback { get; init; }
    /// <summary>meeting.stopped arrived: capture ended and the final pass is running.</summary>
    public bool Stopping { get; init; }
    /// <summary>How far the final pass has come (meeting.transcribed, .diarized, .summarized), while Stopping.</summary>
    public BlotProgress Blotted { get; init; }
    /// <summary>The latest state of each side's capture.</summary>
    public ImmutableDictionary<Channel, SideState> Sides { get; init; } = ImmutableDictionary<Channel, SideState>.Empty;
    /// <summary>Each channel's current partial: replaced by the next one, cleared by its final.</summary>
    public ImmutableDictionary<Channel, string> Partials { get; init; } = ImmutableDictionary<Channel, string>.Empty;
    /// <summary>The newest live finals, oldest first: at most FinalsKept (the record in the store is the lasting copy).</summary>
    public ImmutableList<MeetingFinal> Finals { get; init; } = [];
    /// <summary>What the ledger holds and has let go of.</summary>
    public LedgerStats Ledger { get; init; }

    /// <summary>With <paramref name="final"/> added, letting go of the oldest past FinalsKept.</summary>
    public LiveMeeting Appending(MeetingFinal final)
    {
        var finals = Finals.Add(final);
        var ledger = Ledger with { Seen = Ledger.Seen + 1, Bytes = Ledger.Bytes + Utf8(final.Text) };
        if (finals.Count > FinalsKept)
        {
            var gone = finals[0];
            finals = finals.RemoveAt(0);
            ledger = ledger with { Dropped = ledger.Dropped + 1, Bytes = ledger.Bytes - Utf8(gone.Text) };
        }
        return this with { Finals = finals, Ledger = ledger with { PeakBytes = Math.Max(ledger.PeakBytes, ledger.Bytes) } };
    }

    public bool Equals(LiveMeeting? other) =>
        other is not null && Record == other.Record && Title == other.Title && App == other.App && AppName == other.AppName
        && MicName == other.MicName && MicReason == other.MicReason && FarEnd == other.FarEnd
        && FarEndFallback == other.FarEndFallback && Stopping == other.Stopping && Blotted == other.Blotted
        && Sides.Count == other.Sides.Count && !Sides.Except(other.Sides).Any()
        && Partials.Count == other.Partials.Count && !Partials.Except(other.Partials).Any()
        && Finals.SequenceEqual(other.Finals) && Ledger == other.Ledger;

    public override int GetHashCode() => HashCode.Combine(Record, Stopping, Finals.Count);

    private static int Utf8(string text) => System.Text.Encoding.UTF8.GetByteCount(text);
}

/// <summary>The final pass's steps done so far: each comes once, in this order (diarizing only with several far voices).</summary>
public readonly record struct BlotProgress(bool Transcribed, bool Diarized, bool Summarized)
{
    /// <summary>The sides transcribed so far, 0 to 2 (each reports once): the live icon's ring counts each as a step, as the Mac's does.</summary>
    public int SidesTranscribed => (MicTranscribed ? 1 : 0) + (FarTranscribed ? 1 : 0);

    /// <summary>Your side's final transcription is done.</summary>
    public bool MicTranscribed { get; init; }

    /// <summary>The far end's is.</summary>
    public bool FarTranscribed { get; init; }

    /// <summary>The steps done, in words: "transcribed · speakers sorted · summarized"; null before the first.</summary>
    public string? Words
    {
        get
        {
            var done = new List<string>();
            if (Transcribed)
            {
                done.Add("transcribed");
            }
            if (Diarized)
            {
                done.Add("speakers sorted");
            }
            if (Summarized)
            {
                done.Add("summarized");
            }
            return done.Count == 0 ? null : string.Join(" · ", done);
        }
    }
}

/// <summary>An app the core offers to record.</summary>
public sealed record MeetingOffer(string App, string AppName);

/// <summary>What a notice is about.</summary>
public abstract record NoticeKind
{
    private NoticeKind() { }

    public sealed record CommandFailed(string Command) : NoticeKind;
    public sealed record ModelWarmFailed(Job Job) : NoticeKind;
    public sealed record ModelRefused(Job Job) : NoticeKind;
    public sealed record ModelUpdateFailed(string Model) : NoticeKind;
    public sealed record DictationFailed(FailedStage Stage) : NoticeKind;
    public sealed record DictationWarning(Events.DictationWarning Warning) : NoticeKind;
    public sealed record HotkeyLost : NoticeKind;
    public sealed record EditKeyLost : NoticeKind;
    public sealed record DictationWorkerFailed(bool Recovered) : NoticeKind;
    public sealed record VoiceDetectionUnavailable(VadUnavailable? Reason) : NoticeKind;
    public sealed record AudioDropped(Chain Chain) : NoticeKind;
    public sealed record MeetingWarning(Events.MeetingWarning Warning) : NoticeKind;
    public sealed record MeetingFailed : NoticeKind;
    public sealed record MeetingCaptureFailed : NoticeKind;
    public sealed record MeetingWorkerFailed : NoticeKind;
    /// <summary>A meeting a crash interrupted was finished at launch.</summary>
    public sealed record MeetingRecovered : NoticeKind;
    /// <summary>The meetings a crash may have interrupted could not be looked for.</summary>
    public sealed record RecoveryUnavailable : NoticeKind;
    /// <summary>Detection stopped on its own, or could not start.</summary>
    public sealed record DetectionUnavailable : NoticeKind;
    /// <summary>The retention setting deleted records (a count).</summary>
    public sealed record LibrarySwept(long Deleted, long Failed) : NoticeKind;
    /// <summary>A voice command was heard that this build recognises but does not carry out.</summary>
    public sealed record VoiceCommandNotCarriedOut(CommandAction Action) : NoticeKind;
    /// <summary>An event this shell cannot read: the core and the shell come from different builds.</summary>
    public sealed record MismatchedBuild(string Type) : NoticeKind;
}

/// <summary>Something the user may need to know or act on (the needs-you banner reads these).</summary>
/// <param name="Id">Increases by one per notice.</param>
/// <param name="Detail">The core's own message, when it sent one. Core messages never quote the user's words.</param>
public sealed record Notice(int Id, NoticeKind Kind, string? Detail);

/// <summary>The shell's view of the core. UI thread only; fed by Apply.</summary>
public sealed class CoreStore : ObservableModel
{
    /// <summary>The most notices kept; older ones are dropped first.</summary>
    public const int NoticeLimit = 50;

    private int nextNoticeId = 1;

    /// <summary>Whether the core is running (the status line reads it).</summary>
    public CoreStatus Status { get; private set; } = new(CoreStatusKind.Starting);
    /// <summary>The engines registered, by id, with the jobs they fill.</summary>
    public ImmutableDictionary<string, IReadOnlyList<JobScore>> Engines { get; private set; } =
        ImmutableDictionary<string, IReadOnlyList<JobScore>>.Empty;
    /// <summary>The model kept warm for each job, by id.</summary>
    public ImmutableDictionary<Job, string> WarmModels { get; private set; } = ImmutableDictionary<Job, string>.Empty;
    /// <summary>Models whose files are being replaced.</summary>
    public ImmutableHashSet<string> UpdatingModels { get; private set; } = [];
    public LiveMeeting? Meeting { get; private set; }
    /// <summary>The record of the last meeting that finished.</summary>
    public string? LastRecord { get; private set; }
    /// <summary>The app the core offers to record now, if any.</summary>
    public MeetingOffer? Offer { get; private set; }
    /// <summary>Whether the core listens for calls: null until it says.</summary>
    public bool? Listening { get; private set; }
    /// <summary>The ledger of the meeting that ended last, and its record.</summary>
    public (string Record, LedgerStats Stats)? LastLedger { get; private set; }
    public DictationPhase Dictation { get; private set; } = DictationPhase.Idle;
    /// <summary>The take in progress, while there is one.</summary>
    public LiveDictation? LiveDictation { get; private set; }
    public DictationOutcome? LastDictation { get; private set; }
    public ImmutableList<Notice> Notices { get; private set; } = [];

    /// <summary>Events applied so far (tests and diagnostics).</summary>
    public int EventsApplied { get; private set; }
    /// <summary>Batches applied so far (tests and diagnostics).</summary>
    public int BatchesApplied { get; private set; }

    /// <summary>Folds a batch of events in, oldest first.</summary>
    public void Apply(IReadOnlyList<InkEvent> batch)
    {
        ArgumentNullException.ThrowIfNull(batch);
        foreach (var e in batch)
        {
            Apply(e);
        }
        EventsApplied += batch.Count;
        BatchesApplied++;
        Changed();
    }

    /// <summary>ink_init refused to start (or the shell could not call it). <paramref name="reason"/> is shown.</summary>
    public void StartFailed(string reason)
    {
        Status = new CoreStatus(CoreStatusKind.Failed, reason);
        Changed();
    }

    /// <summary>Removes a notice the user dismissed.</summary>
    public void DismissNotice(int id)
    {
        Notices = Notices.RemoveAll(n => n.Id == id);
        Changed();
    }

    private void Notice(NoticeKind kind, string? detail = null)
    {
        Notices = Notices.Add(new Notice(nextNoticeId++, kind, detail));
        if (Notices.Count > NoticeLimit)
        {
            Notices = Notices.RemoveRange(0, Notices.Count - NoticeLimit);
        }
    }

    private void Apply(InkEvent evt)
    {
        // The status line: ready, stopped, failed on another ABI, or a mismatched build. What it
        // would log is shown here instead: an unreadable event as a notice below, and a failed
        // command by the screen that sent it (or logged by the controller when none shows it).
        Status = Status.Next(evt, _ => { });
        switch (evt)
        {
            // The core
            case CoreStopped:
                Meeting = null;
                Offer = null;
                Listening = null;
                EndDictation();
                break;
            case Events.CommandFailed failed:
                Notice(new NoticeKind.CommandFailed(failed.Command), failed.Message);
                break;

            // Engines and models
            case EngineRegistered engine:
                Engines = Engines.SetItem(engine.Id, engine.Jobs);
                break;
            case EngineUnregistered engine:
                Engines = Engines.Remove(engine.Id);
                break;
            case ModelWarmed warmed:
                WarmModels = WarmModels.SetItem(warmed.Job, warmed.Id);
                break;
            case Events.ModelWarmFailed failed:
                WarmModels = WarmModels.Remove(failed.Job);
                Notice(new NoticeKind.ModelWarmFailed(failed.Job), failed.Message);
                break;
            case Events.ModelRefused refused:
                Notice(new NoticeKind.ModelRefused(refused.Job));
                break;
            case ModelUpdateStarted update:
                UpdatingModels = UpdatingModels.Add(update.Id);
                // Unloaded for the update: no job has it warm until the update warms the new one.
                WarmModels = WarmModels.RemoveRange(WarmModels.Where(w => w.Value == update.Id).Select(w => w.Key).ToList());
                break;
            case ModelUpdateFinished update:
                UpdatingModels = UpdatingModels.Remove(update.Id);
                if (!update.Ok)
                {
                    Notice(new NoticeKind.ModelUpdateFailed(update.Id), update.Message);
                }
                break;
            case Events.AudioDropped dropped:
                Notice(new NoticeKind.AudioDropped(dropped.Chain));
                break;

            // Dictation
            case DictationStarted started:
                Dictation = DictationPhase.Listening;
                LiveDictation = new LiveDictation(started.Take, started.Edit, started.Mode, started.App);
                break;
            case DictationPartial partial:
                // Only the take being held: a late partial of an earlier take is never shown.
                if (Dictation == DictationPhase.Listening && LiveDictation?.Take == partial.Take)
                {
                    LiveDictation = LiveDictation with { Partial = partial.Text };
                }
                break;
            case DictationStopped:
                Dictation = DictationPhase.Transcribing;
                LiveDictation = LiveDictation is null ? null : LiveDictation with { Partial = null };
                break;
            case DictationShortPressIgnored:
                EndDictation();
                break;
            case DictationInserted inserted:
                EndDictation();
                LastDictation = new DictationOutcome.Inserted(inserted.Outcome);
                break;
            case DictationDiscarded discarded:
                EndDictation();
                LastDictation = new DictationOutcome.Discarded(discarded.Reason);
                break;
            case Events.DictationFailed failed:
                EndDictation();
                LastDictation = new DictationOutcome.Failed(failed.Stage);
                Notice(new NoticeKind.DictationFailed(failed.Stage), failed.Message);
                break;
            case DictationCommand command:
                // A voice command ends its take like any other outcome.
                EndDictation();
                // Heard, and nothing typed: never silently.
                if (command.CarriedOut == false)
                {
                    Notice(new NoticeKind.VoiceCommandNotCarriedOut(command.Action));
                }
                break;
            case DictationEdited or DictationEditFailed or DictationMicFailed:
                EndDictation();
                break;
            case DictationEditHotkeyLost:
                Notice(new NoticeKind.EditKeyLost());
                break;
            case DictationVoiceDetection vad when !vad.Available:
                Notice(new NoticeKind.VoiceDetectionUnavailable(vad.Reason));
                break;
            case DictationWarningEvent warning:
                Notice(new NoticeKind.DictationWarning(warning.Kind), warning.Message);
                break;
            case DictationHotkeyLost:
                EndDictation();
                Notice(new NoticeKind.HotkeyLost());
                break;
            case Events.DictationWorkerFailed failed:
                EndDictation();
                Notice(new NoticeKind.DictationWorkerFailed(failed.Recovered));
                break;

            // Meetings
            case MeetingStarted started:
                Meeting = new LiveMeeting(started.Record)
                {
                    Title = started.Title,
                    App = started.App,
                    AppName = AppName(started.App, started.AppName),
                    MicName = started.MicName,
                    MicReason = started.MicReason,
                    FarEnd = started.FarEnd,
                };
                Offer = null;
                break;
            case MeetingDetected detected:
                // While nothing is recorded, or while the last meeting's final pass runs: its
                // capture has ended, so the core offers the next call then (once only). The Drop
                // shows it when the pass ends.
                if (Meeting is null or { Stopping: true })
                {
                    Offer = new MeetingOffer(detected.App, AppName(detected.App, detected.AppName) ?? detected.AppName);
                }
                break;
            case MeetingDetectionEnded ended:
                if (Offer?.App == ended.App)
                {
                    Offer = null;
                }
                break;
            case MeetingDetection detection:
                Listening = detection.Listening;
                if (!detection.Listening)
                {
                    Offer = null;
                    if (detection.Message is not null)
                    {
                        Notice(new NoticeKind.DetectionUnavailable(), detection.Message);
                    }
                }
                break;
            case MeetingFarEndFallback fallback:
                UpdateMeeting(fallback.Record, m => m with { FarEndFallback = true });
                break;
            case Events.MeetingRecovered:
                Notice(new NoticeKind.MeetingRecovered());
                break;
            case MeetingsRecovered done when done.Message is not null:
                Notice(new NoticeKind.RecoveryUnavailable(), done.Message);
                break;
            case Events.LibrarySwept swept:
                Notice(new NoticeKind.LibrarySwept(swept.Deleted, swept.Failed));
                break;
            case MeetingSideState side:
                UpdateMeeting(side.Record, m => m with { Sides = m.Sides.SetItem(side.Channel, side.State) });
                break;
            case MeetingPartial partial:
                UpdateMeeting(partial.Record, m => m with { Partials = m.Partials.SetItem(partial.Channel, partial.Text) });
                break;
            case MeetingFinal final:
                UpdateMeeting(final.Record, m => (m with { Partials = m.Partials.Remove(final.Channel) }).Appending(final));
                break;
            case MeetingStopped stopped:
                UpdateMeeting(stopped.Record, m => m with { Stopping = true, Partials = m.Partials.Clear() });
                break;
            // The final pass's progress (Today's live card shows it while the meeting blots).
            case MeetingTranscribed transcribed:
                UpdateMeeting(transcribed.Record, m => m with
                {
                    Blotted = m.Blotted with
                    {
                        Transcribed = true,
                        MicTranscribed = m.Blotted.MicTranscribed || transcribed.Pass.Channel == Channel.Mic,
                        FarTranscribed = m.Blotted.FarTranscribed || transcribed.Pass.Channel == Channel.Far,
                    },
                });
                break;
            case MeetingDiarized diarized:
                UpdateMeeting(diarized.Record, m => m with { Blotted = m.Blotted with { Diarized = true } });
                break;
            case MeetingSummarized summarized:
                UpdateMeeting(summarized.Record, m => m with { Blotted = m.Blotted with { Summarized = true } });
                break;
            case MeetingVoiceDetection vad when !vad.Available:
                Notice(new NoticeKind.VoiceDetectionUnavailable(vad.Reason));
                break;
            case MeetingWarningEvent warning:
                Notice(new NoticeKind.MeetingWarning(warning.Kind), warning.Message);
                break;
            case MeetingFinished finished:
                LastRecord = finished.Record;
                EndMeeting(finished.Record);
                break;
            case Events.MeetingFailed failed:
                Notice(new NoticeKind.MeetingFailed(), failed.Message);
                // A failure without a record is a meeting that never started.
                if (failed.Record is not null)
                {
                    EndMeeting(failed.Record);
                }
                break;
            case Events.MeetingCaptureFailed:
                Notice(new NoticeKind.MeetingCaptureFailed());
                break;
            case Events.MeetingWorkerFailed failed:
                Notice(new NoticeKind.MeetingWorkerFailed());
                EndMeeting(failed.Record);
                break;
            case UnknownEvent or UndecodableEvent:
                Notice(new NoticeKind.MismatchedBuild(evt.Type));
                break;
            // Nothing to keep: the rest of the final pass, whose screens read the record from the
            // store, and the answers the screens' models take. An event added to the schema later
            // lands here too until the store learns it.
            default:
                break;
        }
    }

    private void EndDictation()
    {
        Dictation = DictationPhase.Idle;
        LiveDictation = null;
    }

    /// <summary>
    /// A meeting app's name as the user knows it. Windows' core names an app by its executable's
    /// stem ("ms-teams"), so a well-known executable gets its app's name ("Microsoft Teams"), as
    /// Settings > Modes names it; any other name stays the core's.
    /// </summary>
    private static string? AppName(string? app, string? coreName) =>
        app is not null && AppIdentity.Known.TryGetValue(app, out var known) ? known : coreName;

    /// <summary>Changes the live meeting when <paramref name="record"/> is the one live; another record's event changes nothing.</summary>
    private void UpdateMeeting(string record, Func<LiveMeeting, LiveMeeting> change)
    {
        if (Meeting is { } live && live.Record == record)
        {
            Meeting = change(live);
        }
    }

    private void EndMeeting(string record)
    {
        if (Meeting is { } live && live.Record == record)
        {
            LastLedger = (record, live.Ledger);
            Meeting = null;
        }
    }
}
