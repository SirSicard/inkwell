// Live: a meeting as it happens, as the Mac's LiveModel. The ledger is read from the CoreStore's
// LiveMeeting (LiveLine.Ledger); this model holds the notes (LiveNotesDraft), the question stack
// and Ask. Nothing ticks here: the header's clock is the view's to redraw, once a second while it
// shows a live meeting, from ElapsedMs.
using System.Collections.Immutable;
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class LiveModel : ObservableModel
{
    /// <summary>
    /// What Live says while nothing has been transcribed yet: waiting for speech, or, with no
    /// speech model installed, that the meeting is recorded but can't be transcribed.
    /// </summary>
    public static string WaitingText(bool noSpeechModel) => noSpeechModel
        ? "This meeting is being recorded, but it can't be transcribed until a speech model is installed. Settings > Models downloads one."
        : "Waiting for someone to speak.";

    private static readonly string[] NoteCommands = ["note.add", "note.update", "note.delete"];

    private readonly Action<CoreCommand> send;
    private readonly Func<DateTimeOffset> now;
    private readonly ScreenLog log;
    private LiveNotesDraft? draft;
    /// <summary>
    /// The meeting stopped while it was being deleted: its notes wait, unsaved, for the delete's
    /// outcome (gone with it, or saved if the core refused the delete).
    /// </summary>
    private bool flushHeld;

    /// <summary>Whether a record is being stopped and deleted (the meetings model's): its notes are never saved.</summary>
    public Func<string, bool> Discarding { get; set; } = _ => false;
    private int nextAsk;
    private string askText = "";

    /// <param name="now">The clock (tests pass a fixed one).</param>
    public LiveModel(Action<CoreCommand> send, Func<DateTimeOffset>? now = null, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
        this.now = now ?? (() => DateTimeOffset.Now);
        this.log = log ?? ScreenLog.System;
    }

    /// <summary>The live meeting's record.</summary>
    public string? Record { get; private set; }
    /// <summary>When this shell heard the meeting start.</summary>
    public DateTimeOffset? StartedAt { get; private set; }
    public FarEndQuestions Stack { get; private set; } = new();
    /// <summary>The questions the user asked, newest first.</summary>
    public ImmutableList<AskedQuestion> Asked { get; private set; } = [];
    /// <summary>The Ask panel shows the newest two.</summary>
    public IEnumerable<AskedQuestion> RecentAsked => Asked.Take(2);
    /// <summary>
    /// The notes' text, as the editor shows it: restored when the screen comes back. The editor
    /// takes it once when it is created (a OneTime binding), never while the user types.
    /// </summary>
    public string NotesText { get; private set; } = "";

    /// <summary>The Ask field's text.</summary>
    public string AskText
    {
        get => askText;
        set
        {
            if (askText != value)
            {
                askText = value ?? "";
                Changed();
            }
        }
    }

    /// <summary>"started 3:04 PM", once the meeting started.</summary>
    public string? StartedLine => StartedAt is DateTimeOffset started
        ? "started " + started.ToLocalTime().ToString("t", CultureInfo.CurrentCulture)
        : null;

    /// <summary>Where the meeting is now (or at <paramref name="at"/>), ms since it started.</summary>
    public long ElapsedMs(DateTimeOffset? at = null)
    {
        if (StartedAt is not DateTimeOffset started)
        {
            return 0;
        }
        var elapsed = (at ?? now()) - started;
        return (long)(Math.Max(0, elapsed.TotalSeconds) * 1_000);
    }

    /// <summary>The header's status: "Blotting…" once capture stopped, else "Recording · 12:41" at <paramref name="at"/>.</summary>
    public string? StatusText(LiveMeeting meeting, DateTimeOffset? at = null)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        if (meeting.Stopping)
        {
            return "Blotting…";
        }
        return StartedAt is null ? null : $"Recording · {LiveClock.Of(ElapsedMs(at))}";
    }

    /// <summary>The notes after an edit: the editor's whole text and the caret's paragraph (null: no caret).</summary>
    public void NotesEdited(string text, int? caretParagraph)
    {
        ArgumentNullException.ThrowIfNull(text);
        NotesText = text;
        if (draft is not null)
        {
            Send(draft.Edited(Paragraphs(text), caretParagraph, (ulong)ElapsedMs()));
        }
        Changed();
    }

    /// <summary>
    /// The editor lost focus: save what is typed. The aggregator's flush before the core stops (the
    /// Mac's flushBeforeStop) calls this too, so a line still under the caret reaches the core.
    /// </summary>
    public void NotesLeft()
    {
        if (draft is not null)
        {
            Send(draft.Flush());
        }
    }

    /// <summary>
    /// The editor's paragraphs: one per line. Any line ending splits them (a WinUI text box writes
    /// "\r"; the Mac's editor "\n").
    /// </summary>
    public static IReadOnlyList<string> Paragraphs(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        return text.ReplaceLineEndings("\n").Split('\n');
    }

    /// <summary>The paragraph the caret at <paramref name="location"/> is in, counting from 0 ("\r\n" is one line ending).</summary>
    public static int ParagraphOf(int location, string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        var upTo = Math.Clamp(location, 0, text.Length);
        var paragraph = 0;
        for (var i = 0; i < upTo; i++)
        {
            if (text[i] == '\n' || (text[i] == '\r' && (i + 1 >= text.Length || text[i + 1] != '\n')))
            {
                paragraph++;
            }
        }
        return paragraph;
    }

    /// <summary>Asks <see cref="AskText"/>.</summary>
    public void SubmitAsk()
    {
        var question = AskText.Trim();
        if (question.Length == 0)
        {
            return;
        }
        askText = "";
        Ask(question);
    }

    /// <summary>Answers the stack's question in <paramref name="slot"/> (Ctrl+1 is the newest).</summary>
    public void AnswerStacked(int slot)
    {
        if (slot < 0 || slot >= Stack.Questions.Count)
        {
            return;
        }
        Ask(Stack.Questions[slot].Text);
    }

    public static string AskRef(int id) => $"ask:{id.ToString(CultureInfo.InvariantCulture)}";

    /// <summary>
    /// Asks the core. The settled lines on screen are not sent: the core answers from the record,
    /// which holds every final, not only the ones the ledger keeps in memory. (The Mac passes them
    /// as an unused context; here they are left out.)
    /// </summary>
    private void Ask(string question)
    {
        var id = nextAsk++;
        Asked = Asked.Insert(0, new AskedQuestion(id, question));
        send(new CoreCommand.MeetingAsk(question, AskRef(id)));
        Changed();
    }

    private void Answered(string? reference, AskAnswer answer)
    {
        var i = reference is null ? -1 : Asked.FindIndex(a => AskRef(a.Id) == reference);
        if (i >= 0)
        {
            Asked = Asked.SetItem(i, Asked[i] with { Answer = answer });
        }
    }

    /// <summary>Whether this model shows the failure (the rest the aggregator logs).</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command == "meeting.ask" || NoteCommands.Contains(failed.Command);
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case MeetingStarted started:
                Record = started.Record;
                StartedAt = now();
                Stack = new FarEndQuestions();
                Asked = [];
                NotesText = "";
                flushHeld = false;
                draft = new LiveNotesDraft(started.Record);
                break;
            case MeetingFinal final when final.Record == Record:
                Stack.Heard(final);
                break;
            case MeetingAnswered answered:
                // Model text: shown as words only.
                Answered(answered.Ref, new AskAnswer.Answer(answered.Text));
                break;
            case CommandFailed failed when failed.Command == "meeting.ask":
                Answered(failed.Id, AskAnswer.Failed(failed.Message));
                break;
            case MeetingStopped stopped when stopped.Record == Record:
                // Capture ended: whatever is typed is saved now, unless the meeting is being deleted
                // (Stop and delete): its words never go into a record that is going.
                if (Discarding(stopped.Record))
                {
                    flushHeld = true;
                }
                else
                {
                    NotesLeft();
                }
                return;
            case CommandFailed failed when failed.Command == "meeting.discard" && flushHeld:
                // The delete was refused after the stop: the meeting is kept, and so are its notes.
                flushHeld = false;
                NotesLeft();
                return;
            case MeetingDiscarded discarded when discarded.Record == Record:
                // Stop and delete: the meeting and its notes are gone with it; nothing is saved to it.
                draft = null;
                End();
                break;
            case NoteAdded added when added.Record == Record:
                if (added.Ref is string addedRef && draft is not null)
                {
                    Send(draft.Added(addedRef, added.Note));
                }
                return;
            case NoteDeleted deleted:
                if (deleted.Ref is string deletedRef && draft is not null)
                {
                    Send(draft.Deleted(deletedRef));
                }
                return;
            case CommandFailed failed when NoteCommands.Contains(failed.Command):
                // Not shown: the line stays unsaved and is sent again when the user next moves on
                // from it. Logged by name only (the note's words are never in a log).
                log.Write($"{failed.Command} failed; the line is saved again when it is left");
                if (failed.Id is string failedRef && draft is not null)
                {
                    Send(draft.Failed(failedRef));
                }
                return;
            case MeetingFinished finished when finished.Record == Record:
                End();
                break;
            case MeetingFailed failed when failed.Record == Record:
                End();
                break;
            case MeetingWorkerFailed failed when failed.Record == Record:
                End();
                break;
            case CoreStopped:
                End();
                break;
            default:
                return;
        }
        Changed();
    }

    private void Send(List<CoreCommand> commands) => commands.ForEach(send);

    private void End()
    {
        // A meeting being deleted keeps none of its notes.
        if (Record is string record && Discarding(record))
        {
            draft = null;
        }
        NotesLeft();
        flushHeld = false;
        Record = null;
        StartedAt = null;
        draft = null;
    }
}
