// The Library's view model (the Mac's LibraryModelTests): its questions carry ids, the newest
// answer wins, and a failed load reads as "couldn't", never as empty.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.LibraryFixtures;

namespace Inkwell.Core.Tests.Screens;

public class LibraryModelTests
{
    private static (LibraryModel Library, Sent Sent) Model(ISearchScheduler? scheduler = null)
    {
        var sent = new Sent();
        return (new LibraryModel(sent.Send, searchScheduler: scheduler), sent);
    }

    /// <summary>Every library command carries an id, and the answer (or its failure) is matched by it.</summary>
    [Fact]
    public void CommandsCarryTheirFieldsAndAnId()
    {
        var list = Fields(new CoreCommand.RecordsList(RecordKind.FileImport, new RecordCursor(7, "r"), 5, "q1"));
        Assert.Equal("records.list", list.GetProperty("cmd").GetString());
        Assert.Equal("file_import", list.GetProperty("kind").GetString());
        Assert.Equal("q1", list.GetProperty("id").GetString());
        Assert.Equal(7, list.GetProperty("before").GetProperty("started_at_unix_ms").GetInt64());
        Assert.False(Fields(new CoreCommand.RecordsList(null, null, 5, "q2")).TryGetProperty("kind", out _));
        foreach (var c in (CoreCommand[])[new CoreCommand.RecordsSearch("x", 1, "a"), new CoreCommand.RecordOpen("r", "b"), new CoreCommand.LibraryStats(0, "c")])
        {
            Assert.False(string.IsNullOrEmpty(RequestId(c)), c.Name);
        }
    }

    /// <summary>
    /// Review fix: a search waits for typing to pause (one question, not one per key), and asks
    /// with at most 200 characters. Windows: the pause is the injected scheduler's, fired by hand
    /// (no sleeping; nothing ticks while idle).
    /// </summary>
    [Fact]
    public void SearchWaitsForTypingToPauseAndIsCapped()
    {
        var scheduler = new ManualSearchScheduler();
        var (library, sent) = Model(scheduler);
        List<CoreCommand> Searches() => sent.Commands.Where(c => Cmd(c) == "records.search").ToList();
        library.Query = "b";
        library.Query = "bu";
        library.Query = "budget";
        Assert.Empty(Searches()); // nothing while typing
        Assert.Equal(LibraryLoad.Loading, library.SearchLoad);
        Assert.Equal(1, scheduler.Waiting);
        Assert.Equal(TimeSpan.FromMilliseconds(250), scheduler.LastDelay);
        scheduler.Fire();
        Assert.Single(Searches());
        Assert.Equal("budget", Fields(Searches()[^1]).GetProperty("query").GetString());

        library.Query = string.Concat(Enumerable.Repeat("budget ", 60));
        scheduler.Fire();
        Assert.Equal(2, Searches().Count);
        var sentQuery = Fields(Searches()[^1]).GetProperty("query").GetString() ?? "";
        Assert.Equal(LibraryModel.MaxQueryLength, sentQuery.Length);
        Assert.StartsWith("budget budget", sentQuery, StringComparison.Ordinal);

        library.Query = "zebra";
        library.Query = "";
        scheduler.Fire();
        Assert.Equal(2, Searches().Count); // cleared before the pause: nothing asked
        Assert.Equal(LibraryLoad.Idle, library.SearchLoad);
    }

    [Fact]
    public void AnAnswerToAnOlderQuestionIsDropped()
    {
        var (library, sent) = Model();
        library.Query = "bud";
        var first = RequestId(sent.Commands[^1]);
        library.Query = "budget";
        var second = RequestId(sent.Commands[^1]);
        library.Apply(Ev.Of($$"""{"type":"library.search","ref":"{{second}}","query":"budget","hits":[{"record":"r","started_at_unix_ms":0,"start_ms":5,"snippet":"the budget"}]}"""));
        library.Apply(Ev.Of($$"""{"type":"library.search","ref":"{{first}}","query":"bud","hits":[]}"""));
        Assert.Equal(["the budget"], library.Hits.Select(h => h.Snippet)); // the stale answer did not replace the newer one
        library.Query = "  ";
        Assert.Empty(library.Hits); // an empty query clears the matches
    }

    [Fact]
    public void OpeningARecordShowsItAndAMissingOneSaysSo()
    {
        var (library, sent) = Model();
        library.Open("r1");
        Assert.Equal("r1", library.Selected);
        Assert.Null(library.Document);
        library.Apply(RecordEvent(RequestId(sent.Commands[^1])));
        Assert.Equal("r1", library.Document?.Record.Record);

        library.Open("gone");
        var id = RequestId(sent.Commands[^1]);
        library.Apply(Ev.Of($$"""{"type":"command.failed","command":"record.open","id":"{{id}}","message":"there is no record gone"}"""));
        Assert.Null(library.Document);
        Assert.Equal("there is no record gone", library.OpenFailure);
    }

    [Fact]
    public void TodayAsksForTheLatestFinishedMeetingThenOpensIt()
    {
        var (library, sent) = Model();
        library.RefreshToday();
        Assert.Equal(["records.list", "library.stats", "library.stats"], sent.Commands.Select(Cmd)); // what is owed is the Owed model's
        var listId = RequestId(sent.Commands[0]);
        library.Apply(Ev.Of($$"""
            {"type":"library.records","ref":"{{listId}}","more":false,"kind":"meeting","records":[
              {{Row("live", start: 9_000)}}, {{Row("old", start: 1_000, end: 2_000)}}, {{Row("r1", start: 5_000, end: 6_000)}}]}
            """));
        var open = Fields(sent.Commands[^1]);
        Assert.Equal("record.open", open.GetProperty("cmd").GetString());
        Assert.Equal("r1", open.GetProperty("record").GetString());
        library.Apply(RecordEvent(open.GetProperty("id").GetString()!));
        Assert.Equal("r1", library.LastMeeting?.Record.Record);
        Assert.Null(library.Document); // Today's record is not the Library's selection
    }

    /// <summary>
    /// Naming a far-end speaker (the Mac's nameSpeaker): on one line and trimmed, sent once as
    /// speaker.name, never when nothing changed (against the last name sent, else the record's),
    /// never for a label the record does not have. When the core answers, the open record is read
    /// again, and Today's last meeting too when it is that record; a refusal is said on the record
    /// until the next try or another record.
    /// </summary>
    [Fact]
    public void NamingASpeakerSendsItOnceAndRereadsWhereTheRecordShows()
    {
        var (library, sent) = Model();
        library.RefreshToday();
        library.Apply(Ev.Of($$"""
            {"type":"library.records","ref":"{{RequestId(sent.Commands[0])}}","more":false,"kind":"meeting","records":[{{Row("r1", start: 5_000, end: 6_000)}}]}
            """));
        library.Apply(RecordEvent(RequestId(sent.Commands[^1])));
        library.Open("r1");
        library.Apply(RecordEvent(RequestId(sent.Commands[^1])));
        var before = sent.Commands.Count;

        library.NameSpeaker("spk1", "  Robin\n  Lee  ");
        var name = Fields(sent.Commands[^1]);
        Assert.Equal("speaker.name", name.GetProperty("cmd").GetString());
        Assert.Equal("r1", name.GetProperty("record").GetString());
        Assert.Equal("spk1", name.GetProperty("speaker").GetString());
        Assert.Equal("Robin Lee", name.GetProperty("name").GetString());
        var id = name.GetProperty("id").GetString()!;

        // Unchanged since it was sent, a label the record lacks, the record's own name: nothing.
        library.NameSpeaker("spk1", "Robin Lee");
        library.NameSpeaker("nobody", "X");
        library.NameSpeaker("spk0", "Alex");
        Assert.Equal(before + 1, sent.Commands.Count);

        library.Apply(Ev.Of($$"""{"type":"speaker.named","record":"r1","speaker":"spk1","named":true,"ref":"{{id}}"}"""));
        var reads = sent.Commands.Skip(before + 1).Select(c => (Cmd(c), Fields(c).GetProperty("record").GetString())).ToList();
        Assert.Equal([("record.open", "r1"), ("record.open", "r1")], reads); // the record, and Today's last meeting

        // A refusal is said on the record, and the next try clears it.
        library.NameSpeaker("spk2", "Sam");
        var refused = RequestId(sent.Commands[^1]);
        library.Apply(Ev.Of($$"""{"type":"command.failed","command":"speaker.name","id":"{{refused}}","message":"the name is longer than 80 characters"}"""));
        Assert.Equal("the name is longer than 80 characters", library.NamingFailure);
        library.NameSpeaker("spk2", "Sam");
        Assert.Null(library.NamingFailure);
        Assert.Equal("speaker.name", Cmd(sent.Commands[^1])); // what was refused is not taken as sent
        library.Open("other");
        Assert.Null(library.NamingFailure);
    }

    /// <summary>A name's length as the core counts it: Unicode scalars, on one line.</summary>
    [Fact]
    public void ANamesLengthIsCountedAsTheCoreCountsIt()
    {
        Assert.Equal(80, LibraryModel.MaxSpeakerName);
        Assert.Equal("a b c", LibraryModel.OneLine(" a\r\n b\tc\u0007"));
        Assert.Equal(3, LibraryModel.NameLength("\U0001F600e\u0301")); // an emoji is one scalar, an accent written as two is two
        Assert.Equal(3, LibraryModel.NameLength(" e\u0301\U0001F600 "));
    }

    [Fact]
    public void ANewRecordOrAMarkedCommitmentRefreshesWhatIsShown()
    {
        var (library, sent) = Model();
        var before = sent.Commands.Count;
        library.Apply(Ev.Of("""{"type":"meeting.finished","record":"r9","revision":2}"""));
        var asked = sent.Commands.Skip(before).Select(Cmd).ToList();
        Assert.True(asked.Contains("records.list") && asked.Contains("library.stats"), string.Join(",", asked));
        library.SetDone("c1", true);
        Assert.Equal("commitment.set_done", Cmd(sent.Commands[^1]));

        // A commitment changed while a record is open: the record is read again.
        library.Open("r1");
        library.Apply(RecordEvent(RequestId(sent.Commands[^1])));
        var reads = sent.Commands.Count;
        library.Apply(Ev.Of("""{"type":"commitment.updated","commitment":"c2","done":true}"""));
        Assert.Equal(reads + 1, sent.Commands.Count);
        Assert.Equal("record.open", Cmd(sent.Commands[^1]));
    }

    /// <summary>
    /// A list or a record that could not be read says so; it never reads as an empty library, and
    /// a stale question's failure changes nothing.
    /// </summary>
    [Fact]
    public void AFailedLoadReadsAsCouldNotLoadNeverAsEmpty()
    {
        var (library, sent) = Model();
        library.RefreshList();
        var first = RequestId(sent.Commands[^1]);
        Assert.Equal(LibraryLoad.Loading, library.ListLoad);
        library.RefreshList();
        var second = RequestId(sent.Commands[^1]);
        static CommandFailed Failed(string id, string command) =>
            Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"{{command}}","id":"{{id}}","message":"the library: disk I/O error"}""");
        library.Apply(Failed(first, "records.list"));
        Assert.Equal(LibraryLoad.Loading, library.ListLoad); // a stale failure changes nothing
        library.Apply(Failed(second, "records.list"));
        Assert.Equal(LibraryLoad.Failed, library.ListLoad);
        Assert.Empty(library.Records);
        Assert.False(library.ShowsEmpty);

        library.RefreshToday();
        var today = RequestId(sent.Commands.Last(c => Cmd(c) == "records.list"));
        library.Apply(Failed(today, "records.list"));
        Assert.Equal(LibraryLoad.Failed, library.LastMeetingLoad); // not "no meetings yet"

        library.Query = "budget";
        library.Apply(Failed(RequestId(sent.Commands[^1]), "records.search"));
        Assert.Equal(LibraryLoad.Failed, library.SearchLoad); // not "nothing matches"
        Assert.Equal("Couldn't search the library.", library.SearchStatus);

        // The screens show these; the aggregator logs the rest by name.
        Assert.True(library.Handles(Failed(second, "records.list")));
        Assert.True(library.Handles(Failed("statsDay-99", "library.stats"))); // Today says the counts could not be read
        Assert.False(library.Handles(Failed("setting:x", "setting.get")));
    }

    /// <summary>Windows addition: the Library column's words (the Mac composes them in LibraryScreen), and the chips toggle as the Mac's do.</summary>
    [Fact]
    public void TheColumnSaysWhatItShows()
    {
        var (library, sent) = Model();
        Assert.Null(library.Filter); // it opens on All
        library.ToggleFilter(RecordKind.Meeting);
        Assert.Equal(RecordKind.Meeting, library.Filter);
        library.ToggleFilter(RecordKind.Meeting);
        Assert.Null(library.Filter); // pressing the one shown shows every kind
        Assert.False(Fields(sent.Commands[^1]).TryGetProperty("kind", out _));
        Assert.Equal("Shows only dictations", library.ChipHint(LibraryModel.Kinds[1]));
        library.ToggleFilter(RecordKind.Dictation);
        Assert.Equal("dictation", Fields(sent.Commands[^1]).GetProperty("kind").GetString());
        Assert.Equal("Shows every kind", library.ChipHint(LibraryModel.Kinds[1]));
        library.Apply(Ev.Of($$"""{"type":"library.records","ref":"{{RequestId(sent.Commands[^1])}}","more":true,"kind":"dictation","records":[{{Row("d1", "dictation", start: 5)}}]}"""));
        Assert.Equal("Library", library.ColumnTitle);
        Assert.Equal("1+", library.CountText);
        Assert.Equal("Show older", library.MoreText);
        library.LoadMore();
        library.Apply(Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"records.list","id":"{{RequestId(sent.Commands[^1])}}","message":"x"}"""));
        Assert.Equal("Couldn't load older records. Try again", library.MoreText);
        Assert.Equal(("No dictations yet", "Each dictation lands here, with the words it typed."), library.EmptyText);
        library.Query = "plan";
        Assert.Equal("Search", library.ColumnTitle);
        Assert.Equal("Searching…", library.SearchStatus);
        library.Apply(Ev.Of($$"""{"type":"library.search","ref":"{{RequestId(sent.Commands[^1])}}","query":"plan","hits":[]}"""));
        Assert.Equal("Nothing said matches “plan”.", library.SearchStatus);
        Assert.Equal("0 matches", library.CountLabel);
    }
}
