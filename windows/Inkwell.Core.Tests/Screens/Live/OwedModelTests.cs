// The Owed screen's model: promises grouped by person or meeting, overdue and "said twice", the
// looks-done suggestions, and what a refused answer says (as the Mac's OwedModelTests and
// OwedRecipientTests).
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class OwedModelTests
{
    private const long Day = 86_400_000;

    private static InkEvent Listed(string items) => Ev.Of($$"""{"type":"commitments.listed","items":[{{items}}]}""");

    [Fact]
    public void PromisesAreGroupedByPersonOrMeetingWithOverdueAndSaidTwice()
    {
        var now = DateTimeOffset.FromUnixTimeSeconds(1_790_500_000);
        var nowMs = now.ToUnixTimeMilliseconds();
        var owed = new OwedModel(_ => { }, TimeZoneInfo.Utc);
        owed.Apply(Listed($$"""
            {"id":"a","record":"r1","record_title":"Planning call","record_started_at_unix_ms":{{nowMs - 3_600_000}},"text":"Share the scorecard draft","due_at_unix_ms":{{nowMs - 2 * Day}},"merged":0},
            {"id":"b","record":"r1","record_title":"Planning call","record_started_at_unix_ms":{{nowMs - 3_600_000}},"text":"Send the pilot deck","due_at_unix_ms":{{nowMs + Day}},"said_at_ms":2332000,"channel":"mic","merged":1},
            {"id":"c","record":"r2","record_started_at_unix_ms":{{nowMs - 5 * Day}},"text":"Review the budget","owner":"Sam","merged":2},
            {"id":"d","record":"r3","record_started_at_unix_ms":{{nowMs - 5 * Day}},"text":"Book a room","merged":0}
            """));
        var groups = owed.Groups(now);
        var fiveDaysAgo = "A meeting on " + OwedDates.WeekdayDayMonth(DateTimeOffset.FromUnixTimeMilliseconds(nowMs - 5 * Day));
        Assert.Equal(["Planning call", "Sam", fiveDaysAgo], groups.Select(g => g.Title));
        Assert.Equal("today", groups[0].Subtitle);
        Assert.Equal(["a", "b"], groups[0].Rows.Select(r => r.Id));
        Assert.Equal(new DueLabel.Overdue(2), groups[0].Rows[0].Due);
        Assert.Equal("2 days overdue", groups[0].Rows[0].Due.Text);
        Assert.Equal(new DueLabel.Tomorrow(), groups[0].Rows[1].Due);
        Assert.Equal("Said twice · merged", groups[0].Rows[1].MergedText);
        Assert.Equal("at 38:52", groups[0].Rows[1].SaidLine);
        Assert.Equal("Said 3 times · merged", groups[1].Rows[0].MergedText);
        Assert.Equal(fiveDaysAgo, groups[1].Rows[0].Meeting);
        Assert.Equal(new DueLabel.Undated(), groups[2].Rows[0].Due);
        Assert.Null(groups[2].Subtitle); // an untitled meeting is named by its day once
        Assert.Equal("4 open · 1 due this week · 1 overdue", owed.Summary(now));
    }

    [Fact]
    public void MarkingDoneTakesItOffAndTheCoresListReplacesIt()
    {
        var sent = new Sent();
        var owed = new OwedModel(sent.Send);
        owed.Apply(Listed("""{"id":"a","record":"r","record_started_at_unix_ms":0,"text":"x","merged":0}"""));
        owed.MarkDone("a");
        Assert.Empty(owed.Items);
        Assert.Equal([new CoreCommand.CommitmentSetDone("a", true)], sent.Commands);
        owed.Apply(Ev.Of("""{"type":"commitment.updated","commitment":"a","done":true}"""));
        Assert.Equal(new CoreCommand.CommitmentsList(), sent.Commands[^1]); // listed again from the core
        owed.Apply(Ev.Of("""{"type":"meeting.commitments","record":"r2","filed":2,"merged":0}"""));
        Assert.Equal(3, sent.Commands.Count); // a meeting filed new ones: listed again
        owed.Apply(Ev.Of("""{"type":"record.deleted","record":"r2","kind":"meeting","audio_left":false,"scrubbed":true}"""));
        Assert.Equal(4, sent.Commands.Count); // a deleted record took its promises: listed again
    }

    /// <summary>Windows only: a list the core could not read says so, never loading or empty (the Mac keeps its spinner).</summary>
    [Fact]
    public void AFailedListSaysSoAndTheNextListClearsIt()
    {
        var owed = new OwedModel(_ => { });
        Assert.True(owed.Loading);
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"commitments.list","message":"the library could not be read"}""");
        Assert.True(OwedModel.Handles(failed));
        owed.Apply(failed);
        Assert.False(owed.Loading);
        Assert.False(owed.Loaded);
        Assert.Equal("Couldn't load what you owe: the library could not be read", owed.LoadFailure);
        owed.Apply(Listed(""));
        Assert.Null(owed.LoadFailure);
        Assert.True(owed.Loaded);
    }
}

public class OwedRecipientTests
{
    private static InkEvent Listed(string items) => Ev.Of($$"""{"type":"commitments.listed","items":[{{items}}]}""");

    /// <summary>Owed groups by the person a promise is owed to, when the meeting said.</summary>
    [Fact]
    public void PromisesGroupByWhoTheyAreOwedTo()
    {
        var owed = new OwedModel(_ => { });
        owed.Apply(Listed("""
            {"id":"c1","record":"r1","record_title":"Partner call","record_started_at_unix_ms":1800000000000,"text":"Send the deck","recipient":"Dana","merged":0},
            {"id":"c2","record":"r2","record_title":"Design review","record_started_at_unix_ms":1800000000000,"text":"Share the notes","recipient":"dana","merged":0},
            {"id":"c3","record":"r1","record_title":"Partner call","record_started_at_unix_ms":1800000000000,"text":"Book the room","merged":0}
            """));
        var groups = owed.Groups(DateTimeOffset.FromUnixTimeSeconds(1_800_000_000));
        Assert.Equal(["To Dana", "Partner call"], groups.Select(g => g.Title));
        Assert.Equal(["c1", "c2"], groups[0].Rows.Select(r => r.Id));
        Assert.Equal(["Partner call", "Design review"], groups[0].Rows.Select(r => r.Meeting));
    }

    /// <summary>"Looks done" suggestions come from the core; "Not yet" tells it.</summary>
    [Fact]
    public void LooksDoneComesFromTheCoreAndNotYetTellsIt()
    {
        var sent = new Sent();
        var owed = new OwedModel(sent.Send);
        owed.Apply(Listed("""
            {"id":"c1","record":"r1","record_started_at_unix_ms":1800000000000,"text":"Send the deck","merged":0,
             "looks_done":{"record":"r9","record_title":"Design review","record_started_at_unix_ms":1800086400000,
             "span":{"channel":"mic","start_ms":1334000,"end_ms":1337000},"text":"I already sent Dana the deck."}}
            """));
        var suggestion = Assert.Single(owed.Suggestions);
        Assert.Equal("c1", suggestion.Commitment);
        Assert.Equal("in Design review you said “I already sent Dana the deck.”", suggestion.Text);
        Assert.Equal("Design review ▸ 22:14", suggestion.Source);
        owed.NotYet(suggestion);
        Assert.Empty(owed.Suggestions);
        Assert.Equal(new CoreCommand.CommitmentNotYet("c1"), sent.Commands[^1]);
        Assert.Equal("""{"cmd":"commitment.not_yet","commitment":"c1"}""", new CoreCommand.CommitmentNotYet("c1").Json);
        owed.Apply(Ev.Of("""{"type":"meeting.looks_done","record":"r9","suggested":1}"""));
        Assert.Equal(new CoreCommand.CommitmentsList(), sent.Commands[^1]); // a meeting found some done: list again
    }

    /// <summary>
    /// A "Mark done" or "Not yet" the core refused puts the promise back and says so, until the
    /// next answer; it is never only a silent reload.
    /// </summary>
    [Fact]
    public void AnAnswerTheCoreRefusedIsSaidAndThePromiseComesBack()
    {
        var sent = new Sent();
        var owed = new OwedModel(sent.Send);
        owed.MarkDone("c1");
        var refused = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"commitment.set_done","message":"the library is read-only"}""");
        Assert.True(OwedModel.Handles(refused));
        owed.Apply(refused);
        Assert.Equal("Couldn't mark it done: the library is read-only", owed.Failure);
        Assert.Equal(new CoreCommand.CommitmentsList(), sent.Commands[^1]); // put back as the core has it
        owed.Apply(Ev.Of("""{"type":"commitments.listed","items":[]}"""));
        Assert.NotNull(owed.Failure); // the reload does not hide it
        owed.NotYet(new LooksDone("looks-done:c2", "c2", "", ""));
        Assert.Null(owed.Failure); // the next answer clears it
        owed.Apply(Ev.Of("""{"type":"command.failed","command":"commitment.not_yet","message":"no such commitment"}"""));
        Assert.Equal("Couldn't keep it open: no such commitment", owed.Failure);
    }
}
