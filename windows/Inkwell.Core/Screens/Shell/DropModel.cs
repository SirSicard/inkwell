// What the Drop says about dictation, and when: the Mac's DropText.dictating, DictationModel.note
// and DropController's order (mac/Sources/Inkwell/ShellInk.swift, Screens/DictationModel.swift,
// Drop.swift), with Windows' words where the cause differs (an app running as administrator, not
// Secure Input; the keyboard hook, not Accessibility).
//
//   a take held or transcribed   its line, the ink dictating (the app in front, its mode, the live words)
//   a take ended not as it should  a note for 2.5 s, the ink still (too short, no mic, not typed...)
//   a note during another take   kept, and shown when that take ends
//   nothing                      the Drop hides
//
// The app's ShellInk draws what this says; nothing here draws, and nothing ticks: a note's end is
// one delayed call. The live words are the user's: shown, never logged. Meetings join this with
// the meetings' end to end.
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

/// <summary>The Drop's two lines.</summary>
/// <param name="LiveWords">The detail is a held take's live words: its end matters, the newest words are wet.</param>
public sealed record DropLine(string Title, string Detail, DropLineTone Tone = DropLineTone.Plain, bool LiveWords = false);

/// <summary>The Drop's words for dictation.</summary>
public static class DictationDrop
{
    /// <summary>
    /// A take in progress: "Dictating · Slack · Chat" (the app in front and its mode) over its live
    /// words, or what it is doing when there are none. Null when no take is in progress.
    /// </summary>
    public static DropLine? Live(DictationPhase phase, LiveDictation? live)
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
        return new(title, "Listening");
    }

    /// <summary>
    /// What the Drop says once for <paramref name="e"/>, or null when it says nothing (the text went
    /// in as it should, or a screen shows it). <paramref name="hasLanguageModel"/>: whether a model
    /// could rewrite a selection, for a failed edit's words.
    /// </summary>
    public static DropLine? Note(InkEvent e, bool hasLanguageModel) => e switch
    {
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
            DictationWarning.ReleaseMissed => new("Stopped after 3 minutes", "The key's release never arrived"),
            // Shown elsewhere (Today's notices, Settings) or nothing the user acts on at once.
            _ => null,
        },
        // Windows removed the keyboard hook and the core could not put it back (Settings says so
        // until dictation is turned on again).
        DictationHotkeyLost => new("The dictation key stopped working", "Turn dictation on again in Settings", DropLineTone.Alert),
        DictationEditHotkeyLost => new("The edit key stopped working", "Turn dictation on again in Settings", DropLineTone.Alert),
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
    private DropLine? live;
    private DropLine? noteShowing;
    private DropLine? noteWaiting;
    private NoteEnd? noteEnd;

    /// <param name="wake">Ends a note after <see cref="NoteDuration"/> (one delayed call per note).</param>
    /// <param name="hasLanguageModel">Whether a language model could rewrite a selection (Polish's engine).</param>
    public DropModel(IWakeScheduler wake, Func<bool>? hasLanguageModel = null)
    {
        ArgumentNullException.ThrowIfNull(wake);
        this.wake = wake;
        this.hasLanguageModel = hasLanguageModel ?? (() => false);
    }

    /// <summary>What the Drop says now; null: it hides.</summary>
    public DropLine? Line => live ?? noteShowing;

    /// <summary>Whether a take is in progress (the ink dictating); a note shows with the ink still.</summary>
    public bool IsLive => live is not null;

    /// <summary>What the Drop shows changed.</summary>
    public event Action? Changed;

    /// <summary>After <paramref name="store"/> has applied <paramref name="batch"/>.</summary>
    public void Apply(CoreStore store, IReadOnlyList<InkEvent> batch)
    {
        ArgumentNullException.ThrowIfNull(store);
        ArgumentNullException.ThrowIfNull(batch);
        var (line, isLive) = (Line, IsLive);
        var wasLive = live is not null;
        live = DictationDrop.Live(store.Dictation, store.LiveDictation);
        foreach (var e in batch)
        {
            if (DictationDrop.Note(e, hasLanguageModel()) is not { } note)
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
        if (Line != line || IsLive != isLive)
        {
            Changed?.Invoke();
        }
    }

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
