// The Live screen's model: the ledger (dry finals, wet partials), the notes sent as note commands
// and matched back by ref, the question stack and Ask, and the header's far-end honesty (as the
// Mac's LiveModelTests, LiveMeetingTests, FarEndHonestyTests and LiveLayoutTests).
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class LiveModelTests
{
    private static void Meeting(LiveModel live) => live.Apply(Ev.Of("""{"type":"meeting.started","record":"rec"}"""));

    private static readonly DateTimeOffset Epoch = DateTimeOffset.FromUnixTimeSeconds(1_000);

    [Fact]
    public void TheLedgerShowsFinalsDryAndPartialsWetAndKeepsNothingOfAPartial()
    {
        var store = new CoreStore();
        store.Apply([
            Ev.Of("""{"type":"meeting.started","record":"rec"}"""),
            Ev.Of("""{"type":"meeting.final","record":"rec","channel":"far","start_ms":738000,"end_ms":741000,"text":"The review takes a week."}"""),
            Ev.Of("""{"type":"meeting.partial","record":"rec","channel":"far","text":"so realistically the"}"""),
            Ev.Of("""{"type":"meeting.partial","record":"rec","channel":"mic","text":"   "}"""),
        ]);
        var lines = LiveLine.Ledger(store.Meeting!);
        Assert.Equal([false, true], lines.Select(l => l.Wet)); // a blank partial is not a line
        // The far end settles a line said earlier than the last one of yours: it goes in its place.
        var mixed = new CoreStore();
        mixed.Apply([
            Ev.Of("""{"type":"meeting.started","record":"r"}"""),
            Ev.Of("""{"type":"meeting.final","record":"r","channel":"mic","start_ms":9000,"end_ms":9500,"text":"b"}"""),
            Ev.Of("""{"type":"meeting.final","record":"r","channel":"far","start_ms":5000,"end_ms":6000,"text":"a"}"""),
            Ev.Of("""{"type":"meeting.final","record":"r","channel":"far","start_ms":9000,"end_ms":9900,"text":"c"}"""),
        ]);
        Assert.Equal(["a", "b", "c"], LiveLine.Ledger(mixed.Meeting!).Select(l => l.Text)); // by time, ties in arrival order
        Assert.Equal(738_000, lines[0].AtMs);
        Assert.Null(lines[1].AtMs);
        Assert.Equal("Them at 12:18: The review takes a week.", lines[0].AccessibilityText);
        Assert.Equal("Them, still settling: so realistically the", lines[1].AccessibilityText);
        store.Apply([Ev.Of("""{"type":"meeting.final","record":"rec","channel":"far","start_ms":741000,"end_ms":744000,"text":"So realistically the fourteenth."}""")]);
        Assert.Equal([false, false], LiveLine.Ledger(store.Meeting!).Select(l => l.Wet)); // the final replaced the wet line
        Assert.Equal("12:18", LiveClock.Of(738_000));
        Assert.Equal("1:02:03", LiveClock.Of(3_723_000));
    }

    [Fact]
    public void TheStackHoldsTheFarEndsQuestionsNewestFirstUpToFour()
    {
        var live = new LiveModel(_ => { });
        Meeting(live);
        void Far(string text, string channel = "far") =>
            live.Apply(Ev.Of($$"""{"type":"meeting.final","record":"rec","channel":"{{channel}}","start_ms":1000,"end_ms":2000,"text":"{{text}}"}"""));
        Far("Can we run the kickoff in parallel? I think so.");
        Far("Could you send the deck by Friday?", "mic");
        Far("Right? What did security say about the data?");
        Far("Can we run the kickoff in parallel?");
        Far("Who owns the rollout plan? When is the review? Where do the models run?");
        Assert.Equal(
            ["Where do the models run?", "When is the review?", "Who owns the rollout plan?", "What did security say about the data?"],
            live.Stack.Questions.Select(q => q.Text));
        Assert.Equal("Ctrl+1", FarEndQuestions.KeyLabel(0)); // the Mac's ⌘1
    }

    [Fact]
    public void AskingSaysPlainlyWhenNothingCanAnswer()
    {
        var live = new LiveModel(_ => { });
        Meeting(live);
        live.AskText = "  What do I owe so far? ";
        live.SubmitAsk();
        Assert.Equal("", live.AskText);
        Assert.Equal("What do I owe so far?", live.Asked[0].Question);
        Assert.Null(live.Asked[0].Answer); // waiting for the core
        Assert.Equal("Thinking…", live.Asked[0].AnswerText);
        // The core has no model to answer with: said in words, never a made-up answer.
        live.Apply(Ev.Of("""{"type":"command.failed","command":"meeting.ask","id":"ask:0","message":"no language model is available to answer on this Mac"}"""));
        Assert.IsType<AskAnswer.Unavailable>(live.Asked[0].Answer);
        Assert.False(live.Asked[0].IsAnswer);
    }

    [Fact]
    public void EachNoteLineIsSavedWhenTheUserMovesOnAndEditsAndDeletesFollowIt()
    {
        var sent = new Sent();
        var now = Epoch;
        var live = new LiveModel(sent.Send, () => now);
        Meeting(live);
        now += TimeSpan.FromSeconds(754);
        live.NotesEdited("Pilot: two teams", 0);
        Assert.Empty(sent.Commands); // the line being typed is not saved yet
        now += TimeSpan.FromSeconds(10);
        live.NotesEdited("Pilot: two teams\n", 1);
        // Stamped with when it was started, not when it was left.
        Assert.Equal([new CoreCommand.NoteAdd("rec", 754_000, "Pilot: two teams", "rec:line:0")], sent.Commands);
        live.NotesEdited("Pilot: two teams\nSecurity review first", 1);
        // The first line is edited before the core answered its add.
        live.NotesEdited("Pilot: two teams, six weeks\nSecurity review first", 1);
        Assert.Single(sent.Commands); // an add on its way is not sent twice
        live.Apply(Ev.Of("""{"type":"note.added","record":"rec","note":"n1","at_ms":754000,"ref":"rec:line:0"}"""));
        Assert.Equal(new CoreCommand.NoteUpdate("n1", "Pilot: two teams, six weeks", "rec:line:0:update"), sent.Commands[^1]);
        // The user leaves the editor: the second line is saved too.
        live.NotesLeft();
        Assert.Equal(new CoreCommand.NoteAdd("rec", 764_000, "Security review first", "rec:line:1"), sent.Commands[^1]);
        live.Apply(Ev.Of("""{"type":"note.added","record":"rec","note":"n2","at_ms":764000,"ref":"rec:line:1"}"""));
        // Deleting the first line deletes its note; the second keeps its own.
        live.NotesEdited("Security review first", 0);
        Assert.Equal(new CoreCommand.NoteDelete("n1", "rec:line:0:delete"), sent.Commands[^1]);
        var before = sent.Commands.Count;
        live.Apply(Ev.Of("""{"type":"meeting.stopped","record":"rec"}"""));
        Assert.Equal(before, sent.Commands.Count); // nothing unsaved is left when capture stops
    }

    /// <summary>One line typed and added as note n1: the line under test.</summary>
    private static LiveModel SavedLine(Sent sent, Logged? logged = null)
    {
        var live = new LiveModel(sent.Send, () => Epoch, logged?.Log);
        Meeting(live);
        live.NotesEdited("Alpha\nBeta", 1);
        live.Apply(Ev.Of("""{"type":"note.added","record":"rec","note":"n1","at_ms":0,"ref":"rec:line:0"}"""));
        sent.Commands.Clear();
        return live;
    }

    [Fact]
    public void AFailingUpdateKeepsTheLineUnsavedAndRetriesOnTheNextLeave()
    {
        var sent = new Sent();
        var logged = new Logged();
        var live = SavedLine(sent, logged);
        live.NotesEdited("Alpha two\nBeta", 1);
        Assert.Equal([new CoreCommand.NoteUpdate("n1", "Alpha two", "rec:line:0:update")], sent.Commands);
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"note.update","id":"rec:line:0:update","message":"the library is busy"}""");
        Assert.True(LiveModel.Handles(failed));
        live.Apply(failed);
        Assert.Single(sent.Commands); // no retry loop: tried again when the user moves on
        // Windows: logged by name, never with the note's words.
        Assert.Equal(["note.update failed; the line is saved again when it is left"], logged.Messages);
        live.NotesLeft();
        Assert.Contains(new CoreCommand.NoteUpdate("n1", "Alpha two", "rec:line:0:update"), sent.Commands.Skip(1)); // the line was not marked saved
        live.Apply(Ev.Of("""{"type":"note.updated","note":"n1","ref":"rec:line:0:update"}"""));
        live.NotesLeft();
        Assert.Equal(2, sent.Commands.Count(c => c.Name == "note.update")); // saved now: nothing more to send
    }

    [Fact]
    public void AFailingDeleteKeepsTheNoteSoARetypeUpdatesAndNeverDuplicates()
    {
        var sent = new Sent();
        var live = SavedLine(sent);
        live.NotesEdited("\nBeta", 1);
        Assert.Equal([new CoreCommand.NoteDelete("n1", "rec:line:0:delete")], sent.Commands);
        // Retyped while the delete is on its way: nothing is sent until the delete settles.
        live.NotesEdited("Alpha again\nBeta", 1);
        Assert.Single(sent.Commands);
        live.Apply(Ev.Of("""{"type":"command.failed","command":"note.delete","id":"rec:line:0:delete","message":"the library is busy"}"""));
        // The note still exists: updated, never added again.
        Assert.Equal(new CoreCommand.NoteUpdate("n1", "Alpha again", "rec:line:0:update"), sent.Commands[^1]);
        Assert.DoesNotContain(sent.Commands, c => c.Name == "note.add");
    }

    [Fact]
    public void AConfirmedDeleteLetsARetypedLineBeAddedAfresh()
    {
        var sent = new Sent();
        var live = SavedLine(sent);
        live.NotesEdited("\nBeta", 1);
        live.NotesEdited("Alpha again\nBeta", 1);
        live.Apply(Ev.Of("""{"type":"note.deleted","note":"n1","ref":"rec:line:0:delete"}"""));
        Assert.Equal(new CoreCommand.NoteAdd("rec", 0, "Alpha again", "rec:line:0"), sent.Commands[^1]);
    }

    [Fact]
    public void ARemovedLinesFailedDeleteIsTriedAgainOnTheNextLeave()
    {
        var sent = new Sent();
        var live = SavedLine(sent);
        live.NotesEdited("Beta", 0);
        Assert.Equal([new CoreCommand.NoteDelete("n1", "rec:line:0:delete")], sent.Commands);
        live.Apply(Ev.Of("""{"type":"command.failed","command":"note.delete","id":"rec:line:0:delete","message":"the library is busy"}"""));
        live.NotesLeft();
        // The note the user deleted does not stay in the record.
        Assert.Equal(2, sent.Commands.Count(c => c == new CoreCommand.NoteDelete("n1", "rec:line:0:delete")));
        live.Apply(Ev.Of("""{"type":"note.deleted","note":"n1","ref":"rec:line:0:delete"}"""));
        live.NotesLeft();
        Assert.Equal(2, sent.Commands.Count(c => c.Name == "note.delete")); // done
    }

    [Fact]
    public void PartialsNeverBecomeNotesOrCommands()
    {
        var sent = new Sent();
        var live = new LiveModel(sent.Send);
        Meeting(live);
        live.Apply(Ev.Of("""{"type":"meeting.partial","record":"rec","channel":"far","text":"so realistically"}"""));
        live.Apply(Ev.Of("""{"type":"meeting.partial","record":"rec","channel":"mic","text":"send the"}"""));
        Assert.Empty(sent.Commands);
    }

    /// <summary>
    /// Windows: the editor is the view's (no TextKit here); the model finds the caret's paragraph
    /// and splits paragraphs on any line ending, since a WinUI text box writes "\r".
    /// </summary>
    [Fact]
    public void TheNotesEditorIsTextKit2()
    {
        Assert.Equal(0, LiveModel.ParagraphOf(0, "a\nb"));
        Assert.Equal(1, LiveModel.ParagraphOf(2, "a\nb"));
        Assert.Equal(2, LiveModel.ParagraphOf(99, "a\nb\n"));
        Assert.Equal(1, LiveModel.ParagraphOf(2, "a\rb"));
        Assert.Equal(1, LiveModel.ParagraphOf(3, "a\r\nb"));
        Assert.Equal(["a", "b", ""], LiveModel.Paragraphs("a\rb\r"));
        Assert.Equal(["a", "b"], LiveModel.Paragraphs("a\r\nb"));
    }

    /// <summary>
    /// The model half of the Mac's CoreControllerCommandTests: a line still under the caret when
    /// the app quits is handed to the core by the flush before stop (NotesLeft). The core half
    /// (the note reaches the record before shutdown returns) needs the real core: not ported.
    /// </summary>
    [Fact]
    public void ANoteStillUnderTheCaretIsSavedWhenTheAppQuits()
    {
        var sent = new Sent();
        var live = new LiveModel(sent.Send, () => Epoch);
        Meeting(live);
        live.NotesEdited("A line still being typed", 0);
        Assert.Empty(sent.Commands); // typed, the caret still in it: nothing handed to the core yet
        live.NotesLeft();
        Assert.Equal([new CoreCommand.NoteAdd("rec", 0, "A line still being typed", "rec:line:0")], sent.Commands);
    }
}

public class LiveMeetingTests
{
    [Fact]
    public void AskGoesToTheCoreAndItsAnswerIsShownAsWords()
    {
        var sent = new Sent();
        var live = new LiveModel(sent.Send);
        live.Apply(Ev.Of("""{"type":"meeting.started","record":"r1"}"""));
        live.AskText = "  What did they say about security? ";
        live.SubmitAsk();
        Assert.Equal(new CoreCommand.MeetingAsk("What did they say about security?", "ask:0"), sent.Commands[^1]);
        Assert.Null(live.Asked[0].Answer); // thinking
        live.Apply(Ev.Of("""{"type":"meeting.answered","record":"r1","ref":"ask:0","text":"They want it reviewed first."}"""));
        Assert.Equal(new AskAnswer.Answer("They want it reviewed first."), live.Asked[0].Answer);

        live.AskText = "What do I owe?";
        live.SubmitAsk();
        live.Apply(Ev.Of("""{"type":"command.failed","command":"meeting.ask","id":"ask:1","message":"no language model is available to answer on this Mac"}"""));
        // Windows: no Apple Intelligence; the model is one the core's engines registered.
        Assert.Equal(new AskAnswer.Unavailable("Answers need a language model, which is off or not ready on this PC."), live.Asked[0].Answer);
        live.AskText = "And now?";
        live.SubmitAsk();
        live.Apply(Ev.Of("""{"type":"command.failed","command":"meeting.ask","id":"ask:2","message":"the model could not answer: engine failed"}"""));
        Assert.Equal(new AskAnswer.Unavailable("Couldn't answer that. Try asking again."), live.Asked[0].Answer);
        Assert.Equal(["And now?", "What do I owe?"], live.RecentAsked.Select(a => a.Question));
    }

    /// <summary>Windows: the header's name is checked here too (LiveHeader), with the store's fields.</summary>
    [Fact]
    public void TheMeetingIsNamedByItsTitleOrItsApp()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1","title":"Weekly sync","app":"example.exe","app_name":"Example Call","mic_name":"Laptop Microphone","mic_transport":"built_in","mic_reason":"built_in_for_bluetooth_output"}""")]);
        var meeting = Assert.IsType<LiveMeeting>(store.Meeting);
        Assert.Equal("Weekly sync", meeting.Title);
        Assert.Equal("Example Call", meeting.AppName);
        Assert.Equal(MicReason.BuiltInForBluetoothOutput, meeting.MicReason);
        Assert.Equal("Weekly sync", LiveHeader.Title(meeting));
        Assert.Equal("Example Call", LiveHeader.AppLine(meeting));
        Assert.Equal("Laptop Microphone, because your headphones are Bluetooth", LiveHeader.MicLine(meeting));
        // Until meeting.started carries a title, the app names it, or "Live meeting".
        Assert.Equal("Example Call call", LiveHeader.Title(meeting with { Title = null }));
        Assert.Null(LiveHeader.AppLine(meeting with { Title = null }));
        Assert.Equal("Live meeting", LiveHeader.Title(new LiveMeeting("r2")));
    }

    [Fact]
    public void TheHeaderSaysWhenASideStoppedOrIsSilentAndWhenCaptureStopped()
    {
        var store = new CoreStore();
        var live = new LiveModel(_ => { }, () => DateTimeOffset.FromUnixTimeSeconds(1_000));
        var started = Ev.Of("""{"type":"meeting.started","record":"r1"}""");
        store.Apply([started]);
        live.Apply(started);
        store.Apply([
            Ev.Of("""{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"""),
            Ev.Of("""{"type":"meeting.side_state","record":"r1","channel":"mic","state":"stopped"}"""),
        ]);
        Assert.Equal(["The others' audio is silent", "Your microphone stopped"], LiveHeader.SideWarnings(store.Meeting!));
        Assert.Equal("Recording · 12:41", live.StatusText(store.Meeting!, DateTimeOffset.FromUnixTimeSeconds(1_000 + 761)));
        store.Apply([Ev.Of("""{"type":"meeting.stopped","record":"r1"}""")]);
        Assert.Equal("Blotting…", live.StatusText(store.Meeting!));
    }
}

public class FarEndHonestyTests
{
    /// <summary>
    /// A call whose app can't be heard alone records everything this PC plays: Live's header says
    /// so throughout. (The Mac's test also checks the ink Drop's text: that is the Drop's, S3.4.)
    /// </summary>
    [Fact]
    public void ACallThatCantBeHeardAloneSaysItRecordsEverything()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1","app":"example.exe","app_name":"Example Call","far_end":"everything"}""")]);
        store.Apply([Ev.Of("""{"type":"meeting.far_end_fallback","record":"r1","app":"example.exe","app_name":"Example Call","message":"no audio process for that app"}""")]);
        var line = Assert.IsType<FarEndLine>(LiveHeader.FarEnd(store.Meeting!));
        Assert.True(line.Alert);
        Assert.Equal("Inkwell couldn't hear Example Call alone, so it is recording everything this PC plays.", line.Text);
        store.Apply([Ev.Of("""{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"shall we start"}""")]);
        Assert.NotNull(LiveHeader.FarEnd(store.Meeting!)); // Live keeps saying it
    }

    /// <summary>Record now records everything this PC plays by design: Live says so, plainly; a call recorded from the offer says nothing more.</summary>
    [Fact]
    public void RecordNowSaysWhatItRecordsAndAnOfferedCallDoesNot()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1","far_end":"everything"}""")]);
        var now = Assert.IsType<FarEndLine>(LiveHeader.FarEnd(store.Meeting!));
        Assert.False(now.Alert);
        Assert.Contains("everything this PC plays", now.Text, StringComparison.Ordinal);
        store.Apply([Ev.Of("""{"type":"meeting.finished","record":"r1","revision":2}""")]);
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r2","app":"a.exe","app_name":"A","far_end":"app"}""")]);
        Assert.Null(LiveHeader.FarEnd(store.Meeting!));
    }
}

/// <summary>
/// The window follows its content's minimum size, so a screen whose minimum grows with its content
/// grows the window off the screen. Windows: the view binds LiveLayout's widths and its notes and
/// ledger scroll inside; the measured check is the view's (the XAML checklist).
/// </summary>
public class LiveLayoutTests
{
    [Fact]
    public void ALongMeetingAndLongNotesNeverRaiseTheLiveScreensMinimumSize()
    {
        var store = new CoreStore();
        var batch = new List<InkEvent> { Ev.Of("""{"type":"meeting.started","record":"rec"}""") };
        for (var i = 0; i < 80; i++)
        {
            var channel = i % 2 == 0 ? "far" : "mic";
            batch.Add(Ev.Of($$"""{"type":"meeting.final","record":"rec","channel":"{{channel}}","start_ms":{{i * 4000}},"end_ms":{{i * 4000 + 3000}},"text":"A settled line of speech, number {{i}}, long enough to wrap onto a second line in the ledger."}"""));
        }
        store.Apply(batch);
        var live = new LiveModel(_ => { });
        live.Apply(batch[0]);
        var minimum = LiveLayout.MinimumWidth;
        live.NotesEdited(string.Join("\n", Enumerable.Repeat("A note line", 60)), 59);
        Assert.Equal(80, LiveLine.Ledger(store.Meeting!).Count);
        Assert.Equal(minimum, LiveLayout.MinimumWidth); // nothing the meeting holds changes it
        Assert.True(LiveLayout.MinimumWidth < LiveLayout.WindowMinWidth, "within the window's minimum content width");
    }
}
