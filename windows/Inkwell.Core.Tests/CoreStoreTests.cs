// The store folds the core's events into what the screens show (as the Mac's CoreStoreTests).
using Inkwell.Core.Events;
using Inkwell.Core.Tests.Screens;
using Xunit;

namespace Inkwell.Core.Tests;

public class CoreStoreTests
{
    private static InkEvent Partial(string text, string channel = "mic", string record = "r1") =>
        Ev.Of($$"""{"type":"meeting.partial","record":"{{record}}","channel":"{{channel}}","text":"{{text}}"}""");

    private static InkEvent Final(int n, string record = "r1") =>
        Ev.Of($$"""{"type":"meeting.final","record":"{{record}}","channel":"mic","start_ms":{{n * 1000}},"end_ms":{{n * 1000 + 900}},"text":"final {{n}}"}""");

    [Fact]
    public void ReadyStoppedAndAMismatchedCore()
    {
        var store = new CoreStore();
        Assert.Equal(CoreStatusKind.Starting, store.Status.Kind);
        store.Apply([Ev.Of($$"""{"type":"core.ready","abi":{{InkSession.AbiVersion}},"version":"1.2.3"}""")]);
        Assert.Equal(new CoreStatus(CoreStatusKind.Ready, "1.2.3"), store.Status);
        store.Apply([Ev.Of("""{"type":"core.stopped"}""")]);
        Assert.Equal(CoreStatusKind.Stopped, store.Status.Kind);

        var mismatched = new CoreStore();
        mismatched.Apply([Ev.Of($$"""{"type":"core.ready","abi":{{InkSession.AbiVersion + 1}},"version":"9.9.9"}""")]);
        Assert.Equal(CoreStatusKind.Failed, mismatched.Status.Kind);
    }

    [Fact]
    public void AMeetingFromStartToFinish()
    {
        var store = new CoreStore();
        store.Apply([
            Ev.Of("""{"type":"meeting.started","record":"r1"}"""),
            Ev.Of("""{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"""),
            Partial("hel"),
            Partial("they sa", "far"),
        ]);
        Assert.Equal("r1", store.Meeting?.Record);
        Assert.Equal(SideState.Zeros, store.Meeting!.Sides[Channel.Far]);
        Assert.Equal("hel", store.Meeting.Partials[Channel.Mic]);
        Assert.Equal("they sa", store.Meeting.Partials[Channel.Far]);

        store.Apply([Final(1)]);
        Assert.False(store.Meeting!.Partials.ContainsKey(Channel.Mic), "a final clears its channel's partial");
        Assert.Single(store.Meeting.Finals);

        store.Apply([Ev.Of("""{"type":"meeting.stopped","record":"r1"}""")]);
        Assert.True(store.Meeting!.Stopping, "blotting: still live until the final pass ends");
        Assert.Empty(store.Meeting.Partials);

        store.Apply([Ev.Of("""{"type":"meeting.finished","record":"r1","revision":2}""")]);
        Assert.Null(store.Meeting);
        Assert.Equal("r1", store.LastRecord);
        Assert.Equal(1, store.LastLedger?.Stats.Seen);
    }

    [Fact]
    public void AnEventForAnotherRecordLeavesTheLiveMeetingAlone()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1"}""")]);
        store.Apply([Partial("stray", record: "r0"), Final(1, "r0")]);
        store.Apply([Ev.Of("""{"type":"meeting.finished","record":"r0"}""")]);
        Assert.Equal(new LiveMeeting("r1"), store.Meeting);
    }

    [Fact]
    public void TheLedgerHoldsAWindowNeverTheSession()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1"}""")]);
        store.Apply(Enumerable.Range(0, LiveMeeting.FinalsKept + 3).Select(n => Final(n)).ToList());
        Assert.Equal(LiveMeeting.FinalsKept, store.Meeting!.Finals.Count);
        Assert.Equal("final 3", store.Meeting.Finals[0].Text);
        Assert.Equal(LiveMeeting.FinalsKept + 3, store.Meeting.Ledger.Seen);
        Assert.Equal(3, store.Meeting.Ledger.Dropped);
        Assert.Equal(store.Meeting.Finals.Sum(f => f.Text.Length), store.Meeting.Ledger.Bytes);
    }

    [Fact]
    public void ADictationFromKeyDownToATooShortTake()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"dictation.started","take":0,"edit":false}""")]);
        Assert.Equal(DictationPhase.Listening, store.Dictation);
        store.Apply([Ev.Of("""{"type":"dictation.stopped"}""")]);
        Assert.Equal(DictationPhase.Transcribing, store.Dictation);
        store.Apply([Ev.Of("""{"type":"dictation.discarded","reason":"speech_too_short"}""")]);
        Assert.Equal(DictationPhase.Idle, store.Dictation);
        Assert.Equal(new DictationOutcome.Discarded(Discard.SpeechTooShort), store.LastDictation);
        Assert.Empty(store.Notices);
    }

    [Fact]
    public void ModelsWarmFailAndUpdate()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"model.warmed","job":"dictation_final","id":"m1"}""")]);
        Assert.Equal("m1", store.WarmModels[Job.DictationFinal]);

        store.Apply([Ev.Of("""{"type":"model.update_started","id":"m1","next":"m2"}""")]);
        Assert.Equal(["m1"], store.UpdatingModels);
        Assert.Empty(store.WarmModels);

        store.Apply([Ev.Of("""{"type":"model.update_finished","id":"m1","next":"m2","ok":false,"no_model_warm":true,"message":"disk full"}""")]);
        Assert.Empty(store.UpdatingModels);
        Assert.Equal(new NoticeKind.ModelUpdateFailed("m1"), Assert.Single(store.Notices).Kind);
        Assert.Equal("disk full", store.Notices[0].Detail);

        store.Apply([Ev.Of("""{"type":"model.warm_failed","job":"dictation_final","message":"not installed"}""")]);
        Assert.Equal(new NoticeKind.ModelWarmFailed(Job.DictationFinal), store.Notices[^1].Kind);
    }

    [Fact]
    public void NoticesAreBoundedOldestFirstAndDismissable()
    {
        var store = new CoreStore();
        store.Apply(Enumerable.Range(0, CoreStore.NoticeLimit + 5)
            .Select(n => Ev.Of($$"""{"type":"command.failed","command":"model.warm","message":"failure {{n}}"}""")).ToList());
        Assert.Equal(CoreStore.NoticeLimit, store.Notices.Count);
        Assert.Equal("failure 5", store.Notices[0].Detail);
        Assert.Equal(store.Notices.Select(n => n.Id).Order(), store.Notices.Select(n => n.Id));

        var first = store.Notices[0].Id;
        store.DismissNotice(first);
        Assert.DoesNotContain(store.Notices, n => n.Id == first);
    }

    [Fact]
    public void AnEventThisBuildCannotReadIsAMismatchedBuild()
    {
        var store = new CoreStore();
        store.Apply([new UnknownEvent { Type = "from.the.future" }]);
        Assert.Equal(new NoticeKind.MismatchedBuild("from.the.future"), Assert.Single(store.Notices).Kind);
    }

    [Fact]
    public void CoreStoppedEndsTheMeetingAndTheDictation()
    {
        var store = new CoreStore();
        store.Apply([
            Ev.Of("""{"type":"meeting.started","record":"r1"}"""),
            Ev.Of("""{"type":"dictation.started","take":0,"edit":false}"""),
            Ev.Of("""{"type":"core.stopped"}"""),
        ]);
        Assert.Null(store.Meeting);
        Assert.Equal(DictationPhase.Idle, store.Dictation);
    }

    [Fact]
    public void OneChangePerBatch()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r1"}"""), Partial("a"), Partial("ab")]);
        Assert.Equal(1, store.Changes);
        Assert.Equal(3, store.EventsApplied);
        Assert.Equal(1, store.BatchesApplied);
    }
}
