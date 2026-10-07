// What the Drop says about meetings, and when (S3.5b; the Mac's DropText.for and DropController's
// order): the offer to record a call while nothing is live, with its two buttons and a failed
// answer said in its place; a meeting recording, its far end in trouble, and its final pass; a
// meeting outranking a take; a note kept until the meeting ends. Every word here is synthetic.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public sealed class MeetingDropTests
{
    private sealed class NoWakes : IWakeScheduler
    {
        public int Scheduled { get; private set; }

        public IDisposable After(TimeSpan delay, Action wake)
        {
            Scheduled++;
            return new Nothing();
        }

        private sealed class Nothing : IDisposable
        {
            public void Dispose()
            {
            }
        }
    }

    private sealed class Rig
    {
        public CoreStore Store { get; } = new();
        public Sent Sent { get; } = new();
        public MeetingModel Meetings { get; }
        public DropModel Drop { get; }
        public int Changes { get; private set; }

        public Rig(Logged? logged = null)
        {
            Meetings = new MeetingModel(Sent.Send, log: (logged ?? new Logged()).Log);
            Drop = new DropModel(new NoWakes(), offerFailure: () => Meetings.FailureOn(MeetingPlace.Drop));
            Drop.Changed += () => Changes++;
        }

        public void Apply(params string[] events)
        {
            var batch = events.Select(Ev.Of).ToList();
            Store.Apply(batch);
            foreach (var e in batch)
            {
                Meetings.Apply(e);
            }
            Drop.Apply(Store, batch);
        }
    }

    private const string Offered = """{"type":"meeting.detected","app":"ms-teams.exe","app_name":"ms-teams"}""";
    private const string Started = """{"type":"meeting.started","record":"r1","app":"ms-teams.exe","app_name":"ms-teams","far_end":"everything"}""";

    private static readonly DropActions TeamsButtons =
        new(new DropAction.Record("ms-teams.exe"), new DropAction.Dismiss("ms-teams.exe"),
            new DropAction.Always("ms-teams.exe", "Microsoft Teams"), new DropAction.Never("ms-teams.exe", "Microsoft Teams"));

    [Fact]
    public void AnAppThatOpensTheMicIsOfferedByItsNameWithTwoButtonsAndTheInkStill()
    {
        var rig = new Rig();
        rig.Apply(Offered);
        // The core names Teams by its executable's stem; the Drop by the app's name.
        Assert.Equal(
            new DropLine("Microsoft Teams opened the microphone", "Recording keeps both sides on this PC. Tell the others you are recording.",
                Actions: TeamsButtons),
            rig.Drop.Line);
        Assert.Equal(DropInk.Idle, rig.Drop.Ink);
        Assert.False(rig.Drop.IsLive);
        Assert.Equal("Record this call", TeamsButtons.First.Title);
        Assert.Equal("Not this one", TeamsButtons.At(1)!.Title);
        Assert.Equal("Always for Microsoft Teams", TeamsButtons.At(2)!.Title);
        Assert.Equal("Never for Microsoft Teams", TeamsButtons.At(3)!.Title);

        // The app lets go of the mic before the user answers: the offer goes.
        rig.Apply("""{"type":"meeting.detection_ended","app":"ms-teams.exe","dismissed":false}""");
        Assert.Null(rig.Drop.Line);
    }

    [Fact]
    public void TheButtonsAnswerThroughTheMeetingsModel()
    {
        var rig = new Rig();
        rig.Apply(Offered);
        rig.Meetings.Perform(rig.Drop.Line!.Actions!.At(0)!);
        Assert.Equal(new CoreCommand.MeetingStart("ms-teams.exe", null), rig.Sent.Commands[^1]);
        rig.Meetings.Perform(rig.Drop.Line!.Actions!.At(1)!);
        Assert.Equal(new CoreCommand.MeetingDismiss("ms-teams.exe"), rig.Sent.Commands[^1]);
        Assert.Null(rig.Drop.Line!.Actions!.At(4));

        rig.Apply("""{"type":"meeting.detection_ended","app":"ms-teams.exe","dismissed":true}""");
        Assert.Null(rig.Drop.Line);
    }

    /// <summary>A failed answer is said on the Drop, in the alert colour; the offer stays to be answered again.</summary>
    [Fact]
    public void AFailedAnswerIsSaidOnTheDropAndTheOfferStays()
    {
        var rig = new Rig();
        rig.Apply(Offered);
        rig.Meetings.Record("ms-teams.exe");
        rig.Apply("""{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"the microphone: no input device"}""");
        var line = rig.Drop.Line!;
        Assert.Equal("Couldn't start recording: the microphone: no input device", line.Detail);
        Assert.Equal(DropLineTone.Alert, line.Tone);
        Assert.Equal(TeamsButtons, line.Actions);

        // Asked again: the failure goes at once (the app refreshes the Drop on the model's change).
        rig.Meetings.Record("ms-teams.exe");
        rig.Drop.Refresh(rig.Store);
        Assert.Equal(DropLineTone.Plain, rig.Drop.Line!.Tone);

        rig.Meetings.Dismiss("ms-teams.exe");
        rig.Apply("""{"type":"command.failed","command":"meeting.dismiss","id":"meeting.dismiss","message":"that app is not being offered"}""");
        Assert.Equal("Couldn't dismiss the offer: that app is not being offered", rig.Drop.Line!.Detail);
    }

    private const string ZoomOffered = """{"type":"meeting.detected","app":"Zoom.exe","app_name":"Zoom"}""";
    private const string Consent = "Recording keeps both sides on this PC. Tell the others you are recording.";

    /// <summary>
    /// Review (S3.5b): a double click on "Record this call" sends two starts; the second fails
    /// after the first has started the meeting. Its offer has gone, so nothing shows it, and the
    /// next call's offer keeps its own consent line.
    /// </summary>
    [Fact]
    public void ASecondClicksFailureIsNotShownOnALaterOffer()
    {
        var logged = new Logged();
        var rig = new Rig(logged);
        rig.Apply(Offered);
        rig.Meetings.Perform(rig.Drop.Line!.Actions!.At(0)!);
        rig.Meetings.Perform(rig.Drop.Line!.Actions!.At(0)!);
        rig.Apply(
            Started,
            """{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"a meeting is already running"}""");
        Assert.Equal(new DropLine("● REC", "Microsoft Teams", DropLineTone.Recording), rig.Drop.Line);
        Assert.Null(rig.Meetings.FailureOn(MeetingPlace.Drop));
        Assert.Contains(logged.Messages, m => m.Contains("its offer had gone", StringComparison.Ordinal));

        rig.Apply("""{"type":"meeting.stopped","record":"r1"}""", """{"type":"meeting.finished","record":"r1","revision":2}""");
        rig.Apply(ZoomOffered);
        Assert.Equal(
            new DropLine("Zoom opened the microphone", Consent,
                Actions: new DropActions(
                    new DropAction.Record("Zoom.exe"), new DropAction.Dismiss("Zoom.exe"),
                    new DropAction.Always("Zoom.exe", "Zoom"), new DropAction.Never("Zoom.exe", "Zoom"))),
            rig.Drop.Line);
    }

    /// <summary>Review (S3.5b): a failed answer goes with its offer, when the app lets go of the mic or another offer comes.</summary>
    [Fact]
    public void AFailedAnswerGoesWithItsOffer()
    {
        var rig = new Rig();
        rig.Apply(Offered);
        rig.Meetings.Record("ms-teams.exe");
        rig.Apply("""{"type":"command.failed","command":"meeting.start","id":"meeting.start","message":"the microphone: no input device"}""");
        Assert.Equal(DropLineTone.Alert, rig.Drop.Line!.Tone);
        rig.Apply("""{"type":"meeting.detection_ended","app":"ms-teams.exe","dismissed":false}""");
        Assert.Null(rig.Drop.Line);
        rig.Apply(ZoomOffered);
        Assert.Equal(Consent, rig.Drop.Line!.Detail);
        Assert.Equal(DropLineTone.Plain, rig.Drop.Line!.Tone);

        // A second offer replacing the first, while its answer's failure shows.
        rig.Meetings.Dismiss("Zoom.exe");
        rig.Apply("""{"type":"command.failed","command":"meeting.dismiss","id":"meeting.dismiss","message":"that app is not being offered"}""");
        Assert.Equal(DropLineTone.Alert, rig.Drop.Line!.Tone);
        rig.Apply(Offered);
        Assert.Equal(new DropLine("Microsoft Teams opened the microphone", Consent, Actions: TeamsButtons), rig.Drop.Line);
    }

    [Fact]
    public void ARecordingSaysRecOverItsAppNeverItsWordsThenItsFinalPass()
    {
        var rig = new Rig();
        rig.Apply(Offered, Started);
        Assert.Equal(DropInk.Meeting, rig.Drop.Ink);
        Assert.True(rig.Drop.IsLive);
        Assert.Equal(new DropLine("● REC", "Microsoft Teams", DropLineTone.Recording), rig.Drop.Line);

        rig.Apply("""{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":" a synthetic line said "}""");
        Assert.Equal(new DropLine("● REC", "Microsoft Teams", DropLineTone.Recording), rig.Drop.Line); // a line said: still the app

        rig.Apply("""{"type":"meeting.stopped","record":"r1"}""");
        Assert.Equal(DropInk.Blotting, rig.Drop.Ink);
        Assert.Equal(new DropLine("Blotting · final pass", "Microsoft Teams"), rig.Drop.Line);

        rig.Apply("""{"type":"meeting.finished","record":"r1","revision":2}""");
        Assert.Equal(DropInk.Idle, rig.Drop.Ink);
        Assert.Null(rig.Drop.Line);
    }

    [Fact]
    public void RecordNowIsNamedByItsTitleAndSaysWhenOtherAppsAreRecordedToo()
    {
        var rig = new Rig();
        rig.Apply("""{"type":"meeting.started","record":"r2","title":"Weekly sync","far_end":"everything"}""");
        Assert.Equal(new DropLine("● REC", "Weekly sync", DropLineTone.Recording), rig.Drop.Line);
        rig.Apply("""{"type":"meeting.stopped","record":"r2"}""");
        Assert.Equal(new DropLine("Blotting · final pass", "Weekly sync"), rig.Drop.Line);

        var fallback = new Rig();
        fallback.Apply(
            """{"type":"meeting.started","record":"r3","app":"Zoom.exe","app_name":"Zoom","far_end":"everything"}""",
            """{"type":"meeting.far_end_fallback","record":"r3","app":"Zoom.exe","app_name":"Zoom","message":"device unavailable: Zoom.exe is not running"}""");
        Assert.Equal(
            "Inkwell couldn't hear Zoom alone, so it is recording everything this PC plays",
            fallback.Drop.Line!.Detail);
    }

    /// <summary>
    /// The far end silent or stopped turns the ink to its problem state, in Windows' words (no
    /// system-audio permission to ask for, so no button); a silent mic stays your drop lying still.
    /// </summary>
    [Fact]
    public void AFarEndInTroubleIsAnAlertAndASilentMicIsNot()
    {
        var rig = new Rig();
        rig.Apply(Started, """{"type":"meeting.side_state","record":"r1","channel":"mic","state":"zeros"}""");
        Assert.Equal(DropInk.Meeting, rig.Drop.Ink);

        rig.Apply("""{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}""");
        Assert.Equal(DropInk.Problem, rig.Drop.Ink);
        Assert.Equal(
            new DropLine("The other side is silent", "Only silence is arriving from the call.", DropLineTone.Alert),
            rig.Drop.Line);
        Assert.Null(rig.Drop.Line!.Actions);

        rig.Apply("""{"type":"meeting.side_state","record":"r1","channel":"far","state":"stopped"}""");
        Assert.Equal("The other side stopped", rig.Drop.Line!.Title);

        rig.Apply("""{"type":"meeting.side_state","record":"r1","channel":"far","state":"ok"}""");
        Assert.Equal(DropInk.Meeting, rig.Drop.Ink);
        Assert.Equal(DropLineTone.Recording, rig.Drop.Line!.Tone);
    }

    [Fact]
    public void AMeetingOutranksATakeAndATakesNoteWaitsForTheMeetingToEnd()
    {
        var rig = new Rig();
        rig.Apply(Started, """{"type":"dictation.started","take":0,"edit":false,"app":"Notepad"}""");
        Assert.Equal(DropInk.Meeting, rig.Drop.Ink);
        Assert.StartsWith("● REC", rig.Drop.Line!.Title, StringComparison.Ordinal);
        rig.Apply("""{"type":"dictation.stopped"}""", """{"type":"dictation.discarded","reason":"too_short"}""");
        Assert.StartsWith("● REC", rig.Drop.Line!.Title, StringComparison.Ordinal);

        rig.Apply("""{"type":"meeting.stopped","record":"r1"}""", """{"type":"meeting.finished","record":"r1","revision":2}""");
        Assert.Equal(new DropLine("Too short", "Try again"), rig.Drop.Line);
        Assert.Equal(DropInk.Idle, rig.Drop.Ink);
    }

    [Fact]
    public void AnOfferWaitsForATakeToEnd()
    {
        var rig = new Rig();
        rig.Apply("""{"type":"dictation.started","take":0,"edit":false}""", Offered);
        Assert.Equal(DropInk.Dictating, rig.Drop.Ink);
        Assert.Equal("Dictating", rig.Drop.Line!.Title);
        rig.Apply("""{"type":"dictation.stopped"}""", """{"type":"dictation.inserted","text":"x","outcome":"pasted"}""");
        Assert.Equal("Microsoft Teams opened the microphone", rig.Drop.Line!.Title);
    }

    /// <summary>
    /// Review (S3.5b): back-to-back calls. The next call is offered while the last meeting's final
    /// pass runs (the core offers once, when its capture has ended): kept, and shown when the pass
    /// ends.
    /// </summary>
    [Fact]
    public void AnOfferDuringTheFinalPassShowsWhenThePassEnds()
    {
        var rig = new Rig();
        rig.Apply(Offered, Started, """{"type":"meeting.stopped","record":"r1"}""");
        rig.Apply(ZoomOffered);
        Assert.Equal(DropInk.Blotting, rig.Drop.Ink);
        Assert.Equal("Blotting · final pass", rig.Drop.Line!.Title);
        Assert.Equal(new MeetingOffer("Zoom.exe", "Zoom"), rig.Store.Offer);

        rig.Apply("""{"type":"meeting.finished","record":"r1","revision":2}""");
        Assert.Equal(DropInk.Idle, rig.Drop.Ink);
        Assert.Equal("Zoom opened the microphone", rig.Drop.Line!.Title);
        Assert.Equal(Consent, rig.Drop.Line!.Detail);

        // While a meeting records, an offer is still not taken.
        rig.Apply(
            """{"type":"meeting.started","record":"r2","app":"Zoom.exe","app_name":"Zoom","far_end":"app"}""",
            Offered);
        Assert.Null(rig.Store.Offer);
    }

    /// <summary>An app the shell does not know keeps the core's name for it.</summary>
    [Fact]
    public void AnUnknownAppKeepsTheCoresName()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.detected","app":"examplecall.exe","app_name":"examplecall"}""")]);
        Assert.Equal(new MeetingOffer("examplecall.exe", "examplecall"), store.Offer);
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"r","app":"zoom.exe","app_name":"zoom"}""")]);
        Assert.Equal("Zoom", store.Meeting!.AppName);
    }
}
