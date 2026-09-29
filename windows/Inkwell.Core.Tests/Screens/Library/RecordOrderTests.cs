// The step's sort-order test, on the shell's side (the Mac's RecordOrderTests): whatever order
// records arrive in, they are shown newest first by start, ties by id; "the last meeting" is the
// latest *finished* one.
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.LibraryFixtures;

namespace Inkwell.Core.Tests.Screens;

public class RecordOrderTests
{
    [Fact]
    public void RecordsShowNewestFirstByStartWhateverOrderTheyArrive()
    {
        var shuffled = Rows($$"""
            {"type":"library.records","more":false,"records":[
              {{Row("b", start: 2_000, end: 3_000)}}, {{Row("d", start: 9_000, end: 9_500)}},
              {{Row("a", start: 1_000, end: 1_500)}}, {{Row("c2", start: 5_000, end: 6_000)}},
              {{Row("c1", start: 5_000, end: 6_000)}}, {{Row("e", "dictation", start: 7_000, end: 7_100)}}]}
            """);
        Assert.Equal(["d", "e", "c2", "c1", "b", "a"], RecordOrder.NewestFirst(shuffled).Select(r => r.Record));
    }

    [Fact]
    public void TheLastMeetingIsTheLatestFinishedOneNotTheLatestWritten()
    {
        var arrived = Rows($$"""
            {"type":"library.records","more":false,"records":[
              {{Row("older", start: 1_000, end: 2_000)}}, {{Row("live", start: 9_000)}},
              {{Row("newest", start: 5_000, end: 6_000)}}, {{Row("dictated", "dictation", start: 8_000, end: 8_100)}}]}
            """);
        Assert.Equal("newest", RecordOrder.LatestFinishedMeeting(arrived)?.Record);
    }

    [Fact]
    public void APageMergesIntoTheListInOrderWithoutRepeats()
    {
        var first = Rows($$"""{"type":"library.records","more":true,"records":[{{Row("x", start: 9)}}, {{Row("y", start: 7)}}]}""");
        var second = Rows($$"""{"type":"library.records","more":false,"records":[{{Row("y", start: 7)}}, {{Row("z", start: 8)}}]}""");
        Assert.Equal(["x", "z", "y"], RecordOrder.NewestFirst(first.Concat(second)).Select(r => r.Record));
    }

    [Fact]
    public void TheModelShowsAnAnswerSortedAndKeepsItSortedAcrossPages()
    {
        var sent = new Sent();
        var library = new LibraryModel(sent.Send);
        library.RefreshList();
        var id = RequestId(sent.Commands[^1]);
        library.Apply(Ev.Of($$"""
            {"type":"library.records","ref":"{{id}}","more":true,"kind":"meeting","records":[
              {{Row("m1", start: 1_000, end: 2_000)}}, {{Row("m3", start: 3_000, end: 4_000)}}, {{Row("m2", start: 2_000, end: 3_000)}}]}
            """));
        Assert.Equal(["m3", "m2", "m1"], library.Records.Select(r => r.Record));
        library.LoadMore();
        var more = Fields(sent.Commands[^1]);
        Assert.Equal("records.list", more.GetProperty("cmd").GetString());
        Assert.Equal("m1", more.GetProperty("before").GetProperty("id").GetString()); // the cursor is the last shown
        library.Apply(Ev.Of($$"""
            {"type":"library.records","ref":"{{more.GetProperty("id").GetString()}}","more":false,"kind":"meeting","records":[{{Row("m0", start: 500, end: 900)}}]}
            """));
        Assert.Equal(["m3", "m2", "m1", "m0"], library.Records.Select(r => r.Record));
        Assert.False(library.HasMore);
    }
}
