// What the Drop says, and when: the Mac's DropText, DictationModel.note and DropController's order
// (mac/Sources/Inkwell/ShellInk.swift, Screens/DictationModel.swift, Drop.swift), with Windows'
// words where the cause differs (an app running as administrator, not Secure Input; the keyboard
// hook, not Accessibility; no system-audio permission, so a silent far end has no button).
//
//   a meeting recording          "● REC · Teams" over its latest line, the ink in meeting (a meeting outranks a take)
//   its far end silent/stopped   an alert, the ink in problem
//   its final pass               "Blotting · final pass", the ink blotting
//   a take held or transcribed   its line, the ink dictating (the app in front, its mode, the live words)
//   a take ended not as it should  a note for 2.5 s, the ink still (too short, no mic, not typed...)
//   a note while something is live  kept, and shown when it ends
//   a personal best just set     a note for 2.5 s, plain ("Longest dictation yet")
//   an app took the mic (the core offers)  "Teams opened the microphone", Record this call / Not this one /
//                                Always for Teams / Never for Teams, the ink still (an app that is Always
//                                already is not offered Always; an Always app asked instead says why)
//   a call Always recorded       "● Recording Teams automatically", the reminder, Stop, and Stop and delete
//                                for its first minute (one wake ends it)
//   Stop and delete pressed      "Stop and delete" / "Deleting this recording" until the core says it is gone
//   nothing                      the Drop hides
//
// The app's ShellInk draws what this says; nothing here draws, and nothing ticks: a note's end is
// one delayed call. The live words and a meeting's lines are the user's: shown, never logged.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>How the Drop colours its title and border.</summary>
public enum DropLineTone
{
    /// <summary>Muted title, hairline border.</summary>
    Plain,
    /// <summary>A recording: the title in seal red.</summary>
    Recording,
    /// <summary>Something needs the user: the title and the border in seal red.</summary>
    Alert,
}

/// <summary>The Drop's two lines, and its buttons when it offers something.</summary>
/// <param name="LiveWords">The detail is a held take's live words: its end matters, the newest words are wet.</param>
/// <param name="Yields">
/// A note that never replaces one showing or waiting (a personal best's: the take's own note, an
/// alert above all, comes first). It is left out then; the Records card still has it.
/// </param>
/// <param name="DetailLines">The most lines the detail takes: three where an Always app asked instead says why.</param>
public sealed record DropLine(
    string Title, string Detail, DropLineTone Tone = DropLineTone.Plain, bool LiveWords = false, DropActions? Actions = null,
    bool Yields = false, int DetailLines = 2);

/// <summary>A button on the Drop.</summary>
public abstract record DropAction
{
    private DropAction() { }

    /// <summary>Record the call the core offered (meeting.start with its app).</summary>
    public sealed record Record(string App) : DropAction;

    /// <summary>"Not this one" (meeting.dismiss).</summary>
    public sealed record Dismiss(string App) : DropAction;

    /// <summary>"Always for Zoom": this call now and every later one without asking (meetings.calls.set, then meeting.start).</summary>
    public sealed record Always(string App, string Name) : DropAction;

    /// <summary>"Never for Zoom": the app's calls are never offered or recorded (meetings.calls.set; the core withdraws the offer).</summary>
    public sealed record Never(string App, string Name) : DropAction;

    /// <summary>Stop a call its app's Always recorded (meeting.stop).</summary>
    public sealed record StopRecording : DropAction;

    /// <summary>"Stop and delete", in that call's first minute (meeting.discard).</summary>
    public sealed record StopAndDelete : DropAction;

    /// <summary>The button's words.</summary>
    public string Title => this switch
    {
        Record => "Record this call",
        Dismiss => "Not this one",
        Always always => $"Always for {MeetingDrop.Named(always.Name)}",
        Never never => $"Never for {MeetingDrop.Named(never.Name)}",
        StopRecording => "Stop",
        StopAndDelete => "Stop and delete",
        _ => throw new InvalidOperationException("a Drop action without words"),
    };

    /// <summary>What Narrator says the button does, beyond its words; null for none.</summary>
    public string? Help => this switch
    {
        Always always => $"Records this call, and from now on records {MeetingDrop.Named(always.Name)}'s calls without asking",
        Never never => $"Inkwell won't offer to record {MeetingDrop.Named(never.Name)}'s calls again",
        StopRecording => "Stops recording; the final pass runs",
        StopAndDelete => "Stops recording and deletes this recording, as if it had never been made",
        _ => null,
    };
}

/// <summary>The Drop's buttons, in order: the first is the answer, drawn in ink.</summary>
public sealed record DropActions(IReadOnlyList<DropAction> All)
{
    public DropActions(params DropAction[] actions)
        : this((IReadOnlyList<DropAction>)actions)
    {
    }

    public DropAction First => All[0];

    public int Count => All.Count;

    /// <summary>The button at <paramref name="index"/>, or null.</summary>
    public DropAction? At(int index) => index >= 0 && index < All.Count ? All[index] : null;

    public bool Equals(DropActions? other) => other is not null && All.SequenceEqual(other.All);

    public override int GetHashCode() => All.Count;
}

/// <summary>What the Drop's ink shows (the app maps it to the renderer's state).</summary>
public enum DropInk
{
    /// <summary>Still: nothing live (a note or an offer may show).</summary>
    Idle,
    Dictating,
    Meeting,
    /// <summary>A meeting's final pass.</summary>
    Blotting,
    /// <summary>A meeting whose far end is silent or stopped.</summary>
    Problem,
}

/// <summary>The Drop's words for a meeting, and for the offer to record one.</summary>
public static class MeetingDrop
{
    /// <summary>
    /// The ink for what is live. A meeting outranks a take. Only the far end sets the problem ink,
    /// which shows the far end's drop gone dead; a silent mic shows as your own drop lying still
    /// (Live and Today say it in words).
    /// </summary>
    public static DropInk Ink(LiveMeeting? meeting, DictationPhase dictation)
    {
        if (meeting is not null)
        {
            if (meeting.Stopping)
            {
                return DropInk.Blotting;
            }
            return meeting.Sides.GetValueOrDefault(Channel.Far, SideState.Ok) == SideState.Ok ? DropInk.Meeting : DropInk.Problem;
        }
        return dictation == DictationPhase.Idle ? DropInk.Idle : DropInk.Dictating;
    }

    /// <summary>What the core calls an app whose name and identity show nothing.</summary>
    public const string Nameless = CallPolicyModel.Nameless;

    /// <summary>The offer's line when an Always app's own sound can't be recorded alone (meeting.detected's message is then the core's NOT_ALONE, word for word).</summary>
    public const string NotAloneMessage = "Inkwell can only record everything this computer plays for this app, so it asks first";

    public const string ConsentLine = "Recording keeps both sides on this PC. Tell the others you are recording.";

    /// <summary>The title names the app, so this line does not: a long name would push the reminder past the last line.</summary>
    public const string NotAloneLine = "Inkwell can't hear it alone: recording takes in everything this PC plays. Tell the others you are recording.";

    /// <summary>An Always app's start that failed (any other meeting.detected message): the platform's words are the core's log's.</summary>
    public const string StartFailedLine = "Inkwell couldn't start recording it by itself. Tell the others you are recording.";

    public const string DiscardingTitle = "Stop and delete";
    public const string DiscardingLine = "Deleting this recording";

    /// <summary>An app's name in a button or a line: "this app" for the core's stand-in for none.</summary>
    public static string Named(string name) => name == Nameless ? "this app" : name;

    public static string AutoTitle(string? name) => $"● Recording {(name is null ? "the call" : Named(name))} automatically";

    public static string AutoReminder(string? name) => $"Always is on for {(name is null ? "this app" : Named(name))}. Tell the others you are recording.";

    private static string Sentence(string text) => text.Length == 0 ? text : string.Concat(text[..1].ToUpperInvariant(), text[1..]);

    /// <summary>
    /// A live meeting's lines for <paramref name="ink"/> (meeting, problem or blotting).
    /// <paramref name="micFallback"/>: the chosen mic isn't connected and another records (said
    /// until the first line); a mic that went mid-call is said over the latest line until a line
    /// comes after it. A call its app's Always recorded says so, keeps the reminder to tell the
    /// others, and offers Stop, and Stop and delete while <paramref name="deletable"/> (its first
    /// minute); <paramref name="discarding"/> once that was pressed. <paramref name="failure"/>: a
    /// Drop button that failed, in words.
    /// </summary>
    public static DropLine Live(
        LiveMeeting meeting, DropInk ink, MicFallback? micFallback = null, bool deletable = false, bool discarding = false,
        string? failure = null)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        if (discarding)
        {
            return new(DiscardingTitle, failure ?? DiscardingLine, failure is null ? DropLineTone.Plain : DropLineTone.Alert);
        }
        var stops = meeting.Auto ? AutoStops(deletable) : null;
        switch (ink)
        {
            case DropInk.Blotting:
                return new("Blotting · final pass", meeting.Title ?? meeting.AppName ?? "The final pass");
            case DropInk.Problem:
                // The plain fact, not a guess at why (muted, a quiet call, another device): Windows
                // has no system-audio permission to ask for, and nothing here is known to fix it. A
                // call its app's Always recorded keeps its Stop (and Stop and delete) here too.
                return meeting.Sides.GetValueOrDefault(Channel.Far, SideState.Ok) == SideState.Zeros
                    ? new("The other side is silent", failure ?? "Only silence is arriving from the call.", DropLineTone.Alert, Actions: stops)
                    : new("The other side stopped", failure ?? "Nothing is arriving from the call. Only your voice may be recorded.", DropLineTone.Alert, Actions: stops);
            case DropInk.Meeting when meeting.Auto:
            {
                var newest = meeting.Finals.Count > 0 ? meeting.Finals[^1].Text.Trim() : "";
                var reminder = AutoReminder(meeting.AppName);
                // A mic that went mid-call is said over everything but a failure, as for any meeting.
                var changed = meeting.MicSwitch is { } micChange && micChange.AtLine == meeting.Ledger.Seen ? SwitchLine(micChange) : null;
                // The reminder holds the first minute, while Stop and delete is there; then the
                // latest line, as any meeting's (before one, the stand-in mic if there is one).
                var detail = changed
                    ?? (deletable ? reminder : newest.Length > 0 ? newest : micFallback is { } standIn ? FallbackLine(standIn) : reminder);
                return new(AutoTitle(meeting.AppName), failure ?? detail, failure is null ? DropLineTone.Recording : DropLineTone.Alert,
                    Actions: stops);
            }
            default:
                var source = meeting.AppName ?? meeting.Title;
                var latest = meeting.Finals.Count > 0 ? meeting.Finals[^1].Text.Trim() : "";
                // Said until the first line arrives: other apps' sound is in this recording.
                var waiting = meeting.FarEndFallback
                    ? $"Inkwell couldn't hear {meeting.AppName ?? "the call"} alone, so it is recording everything this PC plays"
                    : micFallback is { } fallback ? FallbackLine(fallback) : "Recording this meeting";
                var title = source is null ? "● REC" : $"● REC · {source}";
                var switched = meeting.MicSwitch is { } change && change.AtLine == meeting.Ledger.Seen ? SwitchLine(change) : null;
                return new(title, switched ?? (latest.Length > 0 ? latest : waiting), DropLineTone.Recording);
        }
    }

    private static DropActions AutoStops(bool deletable) =>
        deletable ? new DropActions(new DropAction.StopRecording(), new DropAction.StopAndDelete()) : new DropActions(new DropAction.StopRecording());

    /// <summary>"Headset (AirPods Pro) isn't connected. Using Microphone (Realtek)."</summary>
    public static string FallbackLine(MicFallback fallback)
    {
        ArgumentNullException.ThrowIfNull(fallback);
        return $"{fallback.Wanted ?? "Your chosen mic"} isn't connected. Using {fallback.Using}.";
    }

    /// <summary>"Headset (AirPods Pro) went. Now recording with Microphone (Realtek)."</summary>
    public static string SwitchLine(MicSwitch change)
    {
        ArgumentNullException.ThrowIfNull(change);
        return $"{change.From ?? "Your mic"} went. Now recording with {change.To}.";
    }

    /// <summary>
    /// The consent Drop: an app opened the microphone and nothing is live. Honest about what
    /// recording does: both sides are kept on this PC, and the others should be told. A failed
    /// answer (<paramref name="failure"/>) is said in its place; the offer stays, to be answered
    /// again. It also sets the app's policy: "Always for" (unless it is Always already:
    /// <paramref name="policy"/>, or a message saying why an Always app is asked), and "Never for".
    /// </summary>
    public static DropLine Offer(MeetingOffer offer, string? failure, CallPolicy? policy = null)
    {
        ArgumentNullException.ThrowIfNull(offer);
        // An Always app asked instead: why, in the shell's words.
        var line = offer.Message switch
        {
            null => ConsentLine,
            NotAloneMessage => NotAloneLine,
            _ => StartFailedLine,
        };
        var actions = new List<DropAction> { new DropAction.Record(offer.App), new DropAction.Dismiss(offer.App) };
        if (offer.Message is null && policy != CallPolicy.Always)
        {
            actions.Add(new DropAction.Always(offer.App, offer.AppName));
        }
        actions.Add(new DropAction.Never(offer.App, offer.AppName));
        return new(
            Sentence($"{offer.AppName} opened the microphone"),
            failure ?? line,
            failure is null ? DropLineTone.Plain : DropLineTone.Alert,
            Actions: new DropActions(actions),
            DetailLines: offer.Message is null && failure is null ? 2 : 3);
    }
}

/// <summary>The Drop's words for dictation.</summary>
public static class DictationDrop
{
    /// <summary>
    /// A take in progress: "Dictating · Slack · Chat" (the app in front and its mode) over its live
    /// words, or what it is doing when there are none (a stand-in mic for one that isn't connected,
    /// <paramref name="micFallback"/>, while it listens). Null when no take is in progress.
    /// </summary>
    public static DropLine? Live(DictationPhase phase, LiveDictation? live, MicFallback? micFallback = null)
    {
        if (phase == DictationPhase.Idle)
        {
            return null;
        }
        if (live?.Edit == true)
        {
            return new("Editing the selection", phase == DictationPhase.Transcribing ? "Rewriting" : "Say what to change");
        }
        var title = string.Join(" · ", new[] { "Dictating", live?.App, live?.Mode }.Where(s => !string.IsNullOrEmpty(s)));
        if (phase == DictationPhase.Transcribing)
        {
            return new(title, "Transcribing");
        }
        if (live?.Partial is { } words && !string.IsNullOrWhiteSpace(words))
        {
            return new(title, words, LiveWords: true);
        }
        return new(title, micFallback is { } fallback ? MeetingDrop.FallbackLine(fallback) : "Listening");
    }

    /// <summary>
    /// What the Drop says once for <paramref name="e"/>, or null when it says nothing (the text went
    /// in as it should, or a screen shows it). <paramref name="hasLanguageModel"/>: whether a model
    /// could rewrite a selection, for a failed edit's words.
    /// </summary>
    public static DropLine? Note(InkEvent e, bool hasLanguageModel) => Note(e, hasLanguageModel, noSpeechModel: false);

    /// <summary>The line a take ends on while no speech model is installed: why nothing was written, not a guess.</summary>
    public static readonly DropLine NoSpeechModel = new("No speech model is installed", "Settings > Models downloads one", DropLineTone.Alert);

    /// <summary>
    /// As <see cref="Note(InkEvent, bool)"/>; with <paramref name="noSpeechModel"/>, a take that
    /// heard or transcribed nothing says there is no speech model, not that the microphone is
    /// silent or the transcription failed.
    /// </summary>
    public static DropLine? Note(InkEvent e, bool hasLanguageModel, bool noSpeechModel) => e switch
    {
        DictationDiscarded { Reason: Discard.Silence or Discard.NoSpeech or Discard.NothingHeard } when noSpeechModel => NoSpeechModel,
        DictationFailed { Stage: FailedStage.Transcription } when noSpeechModel => NoSpeechModel,
        DictationDiscarded d => d.Reason switch
        {
            Discard.TooShort or Discard.SpeechTooShort => new("Too short", "Try again"),
            Discard.NoSpeech => new("No speech heard", "Nothing was typed"),
            Discard.Silence => new("The microphone is silent", "Check “Hear you” in Settings", DropLineTone.Alert),
            Discard.NothingHeard or Discard.NothingLeft => new("Nothing to type", ""),
            _ => null,
        },
        DictationFailed f => f.Stage switch
        {
            FailedStage.Transcription => new("Couldn't transcribe that", "Nothing was typed", DropLineTone.Alert),
            FailedStage.Insert => new("Couldn't type it here", "Your words are in the Library", DropLineTone.Alert),
            _ => new("Dictation failed", "Nothing was typed", DropLineTone.Alert),
        },
        DictationInserted inserted => Outcome(inserted.Outcome, edit: false),
        DictationEdited edited => Outcome(edited.Outcome, edit: true),
        DictationEditFailed failed => failed.Reason switch
        {
            EditFailure.NoSelection => new("Select some text first", "Then hold the edit key and say what to change"),
            // Windows' "secure input": the app in front runs as administrator, above Inkwell.
            EditFailure.SecureInput => new("Can't edit in this app", "It runs as administrator; the selection was left alone", DropLineTone.Alert),
            // UI Automation gave no selection (the app does not expose one, or did not answer).
            EditFailure.SelectionUnreadable => new("Couldn't read the selection", "This app doesn't share it with Inkwell", DropLineTone.Alert),
            EditFailure.Transcription => new("Couldn't hear the instruction", "The selection was left alone", DropLineTone.Alert),
            EditFailure.NoModel or EditFailure.Model => hasLanguageModel
                ? new("Couldn't rewrite the selection", "It was left alone", DropLineTone.Alert)
                : new("Editing needs a language model", "The selection was left alone", DropLineTone.Alert),
            EditFailure.TimedOut => new("The rewrite took too long", "The selection was left alone", DropLineTone.Alert),
            // Its model now sends somewhere the user has not agreed to (or never agreed).
            EditFailure.NotAllowed => new("Not edited", "Voice edit needs your OK again in Settings", DropLineTone.Alert),
            EditFailure.Insert => new("Couldn't replace the selection", "It was left alone", DropLineTone.Alert),
            _ => new("The edit failed", "The selection was left alone", DropLineTone.Alert),
        },
        // At a press (it would not open) or mid-take (it went away or changed under the take, which
        // then ends with what it heard).
        DictationMicFailed => new("Couldn't open the microphone", "Check “Hear you” in Settings", DropLineTone.Alert),
        DictationWarningEvent w => w.Kind switch
        {
            DictationWarning.PolishTimedOut => new("Polish took too long", "Typed as you said it"),
            // Polish is on, but its model now sends somewhere the user has not agreed to.
            DictationWarning.PolishNotAllowed => new("Not polished", "Polish needs your OK again in Settings", DropLineTone.Alert),
            // The mode names a model of its own that the core does not hold, or that sends elsewhere
            // than where the user agreed: nothing was sent, and Settings > Modes says which.
            DictationWarning.PolishModelMissing => new("Not polished", "Check this mode's model in Settings > Modes", DropLineTone.Alert),
            DictationWarning.ReleaseMissed => new("Stopped after 3 minutes", "The key's release never arrived"),
            // Shown elsewhere (Today's notices, Settings) or nothing the user acts on at once.
            _ => null,
        },
        // Windows removed the keyboard hook and the core could not put it back (Settings says so
        // until dictation is turned on again).
        DictationHotkeyLost => new("The dictation key stopped working", "Turn dictation on again in Settings", DropLineTone.Alert),
        DictationEditHotkeyLost => new("The edit key stopped working", "Turn dictation on again in Settings", DropLineTone.Alert),
        // A best the take (or today, or this week) just set: the core reports it once, never for
        // an import, and never with celebrations off. A note to read, not an alert.
        MilestonesReached { Best: { } best } => StatsFormat.BestNote(best),
        _ => null,
    };

    private static DropLine? Outcome(InsertOutcome outcome, bool edit) => outcome switch
    {
        // The app in front runs as administrator: Windows drops input from Inkwell there.
        InsertOutcome.Blocked => new(
            edit ? "Can't edit in this app" : "Can't type into this app",
            edit ? "It runs as administrator; the selection was left alone" : "It runs as administrator; your words are in the Library",
            DropLineTone.Alert),
        InsertOutcome.InsertedClipboardNotRestored => new(edit ? "Replaced" : "Typed", "Your clipboard couldn't be put back"),
        _ => null,
    };
}

/// <summary>
/// Whether the Drop shows, what it says, and whether its ink is live, from the store after each
/// batch. UI thread only.
/// </summary>
public sealed class DropModel
{
    /// <summary>How long a note stays up.</summary>
    public static readonly TimeSpan NoteDuration = TimeSpan.FromMilliseconds(2_500);

    private readonly IWakeScheduler wake;
    private readonly Func<bool> hasLanguageModel;
    private readonly Func<bool> noSpeechModel;
    private readonly Func<string?> offerFailure;
    private readonly Func<string, bool> deletable;
    private readonly Func<string, bool> discarding;
    private readonly Func<string, CallPolicy?> policyOf;
    private DropLine? live;
    private DropLine? noteShowing;
    private DropLine? noteWaiting;
    private DropLine? offer;
    private NoteEnd? noteEnd;

    /// <param name="wake">Ends a note after <see cref="NoteDuration"/> (one delayed call per note).</param>
    /// <param name="hasLanguageModel">Whether a language model could rewrite a selection (Polish's engine).</param>
    /// <param name="offerFailure">A Drop answer that failed, in words (the meetings model's), or null.</param>
    /// <param name="noSpeechModel">Whether no speech model is installed (the catalogue's answer).</param>
    /// <param name="deletable">Whether Stop and delete is offered for a record now (the meetings model's).</param>
    /// <param name="discarding">Whether a record is being stopped and deleted (the meetings model's).</param>
    /// <param name="policyOf">An app's call policy, when known (the call policies' model).</param>
    public DropModel(
        IWakeScheduler wake, Func<bool>? hasLanguageModel = null, Func<string?>? offerFailure = null, Func<bool>? noSpeechModel = null,
        Func<string, bool>? deletable = null, Func<string, bool>? discarding = null, Func<string, CallPolicy?>? policyOf = null)
    {
        ArgumentNullException.ThrowIfNull(wake);
        this.wake = wake;
        this.hasLanguageModel = hasLanguageModel ?? (() => false);
        this.noSpeechModel = noSpeechModel ?? (() => false);
        this.offerFailure = offerFailure ?? (() => null);
        this.deletable = deletable ?? (_ => false);
        this.discarding = discarding ?? (_ => false);
        this.policyOf = policyOf ?? (_ => null);
    }

    /// <summary>What the Drop says now; null: it hides. What is live first, then a note, then an offer.</summary>
    public DropLine? Line => live ?? noteShowing ?? offer;

    /// <summary>Whether something is live (a meeting or a take); a note or an offer shows with the ink still.</summary>
    public bool IsLive => live is not null;

    /// <summary>What the Drop's ink shows now.</summary>
    public DropInk Ink { get; private set; }

    /// <summary>What the Drop shows changed.</summary>
    public event Action? Changed;

    /// <summary>After <paramref name="store"/> has applied <paramref name="batch"/>.</summary>
    public void Apply(CoreStore store, IReadOnlyList<InkEvent> batch)
    {
        ArgumentNullException.ThrowIfNull(store);
        ArgumentNullException.ThrowIfNull(batch);
        var (line, ink) = (Line, Ink);
        var wasLive = live is not null;
        Ink = MeetingDrop.Ink(store.Meeting, store.Dictation);
        live = LiveLine(store);
        // The core offers only while nothing is being captured; what is live (a take, or the last
        // meeting's final pass) hides the offer until it ends.
        offer = OfferLine(store);
        foreach (var e in batch)
        {
            if (DictationDrop.Note(e, hasLanguageModel(), noSpeechModel()) is not { } note
                || (note.Yields && (noteShowing is not null || noteWaiting is not null)))
            {
                continue;
            }
            if (live is not null)
            {
                noteWaiting = note;
            }
            else
            {
                noteWaiting = null;
                ShowNote(note);
            }
        }
        if (live is not null)
        {
            // What is live comes first; a note that was up is done.
            EndNote();
        }
        else if (wasLive && noteWaiting is { } waiting)
        {
            noteWaiting = null;
            ShowNote(waiting);
        }
        if (Line != line || Ink != ink)
        {
            Changed?.Invoke();
        }
    }

    /// <summary>
    /// The offer's line again, for a change outside a batch (an answer sent again clears its
    /// failure). What is live and the notes stay as the last batch left them.
    /// </summary>
    public void Refresh(CoreStore store)
    {
        ArgumentNullException.ThrowIfNull(store);
        var line = Line;
        if (live is not null && store.Meeting is not null)
        {
            // A meeting's buttons and failure (Stop and delete's minute ending, a press, a refusal).
            live = LiveLine(store);
        }
        offer = OfferLine(store);
        if (Line != line)
        {
            Changed?.Invoke();
        }
    }

    private DropLine? LiveLine(CoreStore store) => Ink switch
    {
        DropInk.Idle => null,
        DropInk.Dictating => DictationDrop.Live(store.Dictation, store.LiveDictation, store.MicFallback),
        _ => MeetingDrop.Live(
            store.Meeting!, Ink, store.MicFallback, deletable(store.Meeting!.Record), discarding(store.Meeting!.Record),
            store.Meeting!.Auto || discarding(store.Meeting!.Record) ? offerFailure() : null),
    };

    private DropLine? OfferLine(CoreStore store) =>
        live is null && store.Offer is { } offered ? MeetingDrop.Offer(offered, offerFailure(), policyOf(offered.App)) : null;

    private void ShowNote(DropLine note)
    {
        EndNote();
        noteShowing = note;
        var end = new NoteEnd();
        end.Timer = wake.After(NoteDuration, () =>
        {
            // Only this note's end: a later note has its own.
            if (!ReferenceEquals(noteEnd, end))
            {
                return;
            }
            noteEnd = null;
            noteShowing = null;
            end.Timer?.Dispose();
            Changed?.Invoke();
        });
        noteEnd = end;
    }

    private void EndNote()
    {
        noteEnd?.Timer?.Dispose();
        noteEnd = null;
        noteShowing = null;
    }

    private sealed class NoteEnd
    {
        public IDisposable? Timer { get; set; }
    }
}
