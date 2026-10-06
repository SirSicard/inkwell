// Per-app call recording on the Drop, as the Mac's CallPolicyDropTests: the offer sets an app's
// policy (Always for, Never for), a call its app's Always recorded shows as such with Stop, and Stop
// and delete for its first minute (one wake ends it), and an Always app the core asks about instead
// says why. Every word here is synthetic.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public sealed class CallPolicyDropTests
{
    /// <summary>The wakes a model schedules, held until the test lets them pass.</summary>
    private sealed class HeldWakes : IWakeScheduler
    {
        public List<(TimeSpan Delay, Action Wake, Wait Handle)> Scheduled { get; } = [];

        public int Pending => Scheduled.Count(s => !s.Handle.Disposed);

        public IDisposable After(TimeSpan delay, Action wake)
        {
            var handle = new Wait();
            Scheduled.Add((delay, wake, handle));
            return handle;
        }

        /// <summary>Every wake not cancelled runs, as their moments passing would.</summary>
        public void Fire()
        {
            foreach (var (_, wake, handle) in Scheduled.ToList())
            {
                if (!handle.Disposed && !handle.Fired)
                {
                    handle.Fired = true;
                    wake();
                }
            }
        }

        public sealed class Wait : IDisposable
        {
            public bool Disposed { get; private set; }
            public bool Fired { get; set; }
            public void Dispose() => Disposed = true;
        }
    }

    /// <summary>The app's wiring: the store, the screens, and the Drop over them.</summary>
    private sealed class Rig
    {
        public CoreStore Store { get; } = new();
        public Sent Sent { get; } = new();
        public Logged Logged { get; } = new();
        public HeldWakes Wakes { get; } = new();
        public ScreenModels Screens { get; }
        public DropModel Drop { get; }

        public Rig()
        {
            Screens = new ScreenModels(Sent.Send, wake: Wakes, log: Logged.Log);
            Drop = new DropModel(
                new HeldWakes(), offerFailure: () => Screens.DropFailure,
                deletable: record => Screens.Meetings.CanDiscard(record),
                discarding: record => Screens.Meetings.Discarding == record,
                policyOf: app => Screens.Calls.PolicyOf(app));
            // As the app: a change outside a batch (a wake, a press) shows at once; one during a
            // batch is the batch's.
            Screens.Meetings.PropertyChanged += (_, _) => Refresh();
            Screens.Calls.PropertyChanged += (_, _) => Refresh();
        }

        private bool applying;

        private void Refresh()
        {
            if (!applying)
            {
                Drop.Refresh(Store);
            }
        }

        public void Apply(params string[] events)
        {
            var batch = events.Select(Ev.Of).ToList();
            Store.Apply(batch);
            applying = true;
            try
            {
                Screens.Apply(batch);
            }
            finally
            {
                applying = false;
            }
            Drop.Apply(Store, batch);
        }

        public DropLine Line => Drop.Line!;

        public IReadOnlyList<string> Titles => Line.Actions?.All.Select(a => a.Title).ToList() ?? [];

        /// <summary>The ref of the last call-policy set sent.</summary>
        public string LastSetRef => Sent.Commands.OfType<CoreCommand.MeetingsCallsSet>().Last().Ref;
    }

    private const string Zoom = "zoom.exe";
    private const string Offered = """{"type":"meeting.detected","app":"zoom.exe","app_name":"zoom"}""";

    private static string Detected(string app = Zoom, string name = "zoom", string? message = null) =>
        message is null
            ? $$"""{"type":"meeting.detected","app":"{{app}}","app_name":"{{name}}"}"""
            : $$"""{"type":"meeting.detected","app":"{{app}}","app_name":"{{name}}","message":"{{message}}"}""";

    private static string Started(bool auto, DateTimeOffset? until, string record = "r1") =>
        $$"""{"type":"meeting.started","record":"{{record}}","app":"zoom.exe","app_name":"zoom","far_end":"app"{{(auto ? ",\"auto\":true" : "")}}{{(until is { } u ? $",\"delete_until_unix_ms\":{u.ToUnixTimeMilliseconds()}" : "")}}}""";

    private static string Calls(string apps, string policy = "ask", string? message = null, string? @ref = null) =>
        $$"""{"type":"meetings.calls","default":"{{policy}}","apps":[{{apps}}]{{(message is null ? "" : $",\"message\":\"{message}\"")}}{{(@ref is null ? "" : $",\"ref\":\"{@ref}\"")}}}""";

    private static DateTimeOffset InAMinute => DateTimeOffset.Now.AddMinutes(1);

    [Fact]
    public void TheOfferSetsTheAppsPolicyFromTheDrop()
    {
        var rig = new Rig();
        rig.Apply(Offered);
        Assert.Equal("Zoom opened the microphone", rig.Line.Title);
        Assert.Equal(MeetingDrop.ConsentLine, rig.Line.Detail);
        Assert.Equal(["Record this call", "Not this one", "Always for Zoom", "Never for Zoom"], rig.Titles);
        Assert.Equal(2, rig.Line.DetailLines);
        Assert.NotNull(rig.Line.Actions!.At(2)!.Help); // Narrator hears what Always does

        // Always for Zoom: saved first; this call is recorded once the core says so.
        rig.Screens.PerformDropAction(rig.Line.Actions!.At(2)!);
        Assert.Equal(new CoreCommand.MeetingsCallsSet(Zoom, "always", false, rig.LastSetRef), rig.Sent.Commands[^1]);
        rig.Apply(Calls("""{"app":"zoom.exe","app_name":"zoom","policy":"always","chosen":true}""", @ref: rig.LastSetRef));
        Assert.Equal(new CoreCommand.MeetingStart(Zoom, null), rig.Sent.Commands[^1]);

        // Never for Zoom: only the policy; the core withdraws the offer.
        rig.Apply(Offered);
        rig.Screens.PerformDropAction(new DropAction.Never(Zoom, "Zoom"));
        Assert.Equal(new CoreCommand.MeetingsCallsSet(Zoom, "never", false, rig.LastSetRef), rig.Sent.Commands[^1]);
        Assert.Equal(
            """{"app":"zoom.exe","cmd":"meetings.calls.set","id":"calls:9","policy":"never"}""",
            new CoreCommand.MeetingsCallsSet(Zoom, "never", false, "calls:9").Json);
        Assert.Equal(
            """{"app":"zoom.exe","cmd":"meetings.calls.set","id":"calls:9","policy":"ask","replace_unreadable":true}""",
            new CoreCommand.MeetingsCallsSet(Zoom, "ask", true, "calls:9").Json);
        Assert.Equal("""{"cmd":"meetings.calls.list","id":"calls:9"}""", new CoreCommand.MeetingsCallsList("calls:9").Json);
        Assert.Equal("""{"cmd":"meeting.discard","id":"meeting.discard"}""", new CoreCommand.MeetingDiscard().Json);
    }

    [Fact]
    public void AnAppAlreadyAlwaysIsNotOfferedAlwaysAgain()
    {
        var rig = new Rig();
        rig.Apply(Calls("""{"app":"zoom.exe","app_name":"zoom","policy":"always","chosen":true}"""), Offered);
        Assert.Equal(["Record this call", "Not this one", "Never for Zoom"], rig.Titles);
    }

    /// <summary>NotAlone (Windows' loopback, e.g. Teams): asked about, saying why in the shell's words, with the reminder.</summary>
    [Fact]
    public void AnAlwaysAppThatCantBeHeardAloneIsAskedAboutAndSaysWhy()
    {
        var rig = new Rig();
        rig.Apply(Detected("ms-teams.exe", "ms-teams", MeetingDrop.NotAloneMessage));
        Assert.Equal("Microsoft Teams opened the microphone", rig.Line.Title);
        Assert.Equal("Inkwell can't hear it alone: recording takes in everything this PC plays. Tell the others you are recording.", rig.Line.Detail);
        Assert.Equal(3, rig.Line.DetailLines);
        Assert.Equal(["Record this call", "Not this one", "Never for Microsoft Teams"], rig.Titles);
        rig.Apply(Detected("ms-teams.exe", "ms-teams", "couldn't start recording by itself: the loopback failed"));
        Assert.Equal(MeetingDrop.StartFailedLine, rig.Line.Detail);
        foreach (var word in new[] { "invisible", "undetectable", "hidden", "secret", "Mac" })
        {
            Assert.DoesNotContain(word, rig.Line.Title + rig.Line.Detail, StringComparison.OrdinalIgnoreCase);
        }
    }

    [Fact]
    public void ANamelessAppReadsAsThisApp()
    {
        var rig = new Rig();
        rig.Apply(Detected("examplex.exe", "an app"));
        Assert.Equal("An app opened the microphone", rig.Line.Title);
        Assert.Equal(["Always for this app", "Never for this app"], rig.Titles.Skip(2));
    }

    /// <summary>A call its app's Always started: said as such, the reminder kept, Stop and Stop and delete for the first minute, ended by one wake.</summary>
    [Fact]
    public void AnAutoStartedCallShowsStopAndDeleteUntilItsDeadline()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, InAMinute));
        Assert.Equal("● Recording Zoom automatically", rig.Line.Title);
        Assert.Equal("Always is on for Zoom. Tell the others you are recording.", rig.Line.Detail);
        Assert.Equal(DropLineTone.Recording, rig.Line.Tone);
        Assert.Equal(["Stop", "Stop and delete"], rig.Titles);
        Assert.Single(rig.Wakes.Scheduled); // one deadline, never a poll
        Assert.InRange(rig.Wakes.Scheduled[0].Delay, TimeSpan.FromSeconds(55), TimeSpan.FromSeconds(61));

        rig.Apply("""{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"can everyone hear me"}""");
        Assert.Equal("Always is on for Zoom. Tell the others you are recording.", rig.Line.Detail); // the reminder holds the minute

        rig.Wakes.Fire();
        Assert.Equal(["Stop"], rig.Titles); // past the minute only Stop is left
        Assert.Equal("can everyone hear me", rig.Line.Detail);
        Assert.Equal("● Recording Zoom automatically", rig.Line.Title);
        Assert.Single(rig.Wakes.Scheduled);

        rig.Screens.PerformDropAction(rig.Line.Actions!.At(0)!);
        Assert.Equal(new CoreCommand.MeetingStop(), rig.Sent.Commands[^1]);
    }

    [Fact]
    public void ANewMeetingOrAStopCancelsThePendingWake()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, InAMinute, "r1"));
        rig.Apply(Started(auto: true, InAMinute, "r2"));
        Assert.Equal(2, rig.Wakes.Scheduled.Count);
        Assert.Equal(1, rig.Wakes.Pending); // the first was cancelled
        Assert.False(rig.Screens.Meetings.CanDiscard("r1"));
        Assert.True(rig.Screens.Meetings.CanDiscard("r2"));
        rig.Apply("""{"type":"meeting.stopped","record":"r2"}""");
        Assert.False(rig.Screens.Meetings.CanDiscard("r2"));
        Assert.Equal(0, rig.Wakes.Pending);
    }

    [Fact]
    public void AUserStartedCallHasNoStopAndDelete()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: false, InAMinute));
        Assert.Equal("● REC · Zoom", rig.Line.Title);
        Assert.Null(rig.Line.Actions);
        Assert.Empty(rig.Wakes.Scheduled);
    }

    [Fact]
    public void ADeadlineAlreadyPastOffersNoDelete()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, DateTimeOffset.Now.AddSeconds(-1)));
        Assert.Equal(["Stop"], rig.Titles);
        Assert.Empty(rig.Wakes.Scheduled);
    }

    /// <summary>Stop and delete: the Drop says it is deleting through the stop; gone when the core says so, never the last record.</summary>
    [Fact]
    public void StopAndDeleteDeletesAndTheDropGoes()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, InAMinute));
        rig.Screens.PerformDropAction(rig.Line.Actions!.At(1)!);
        Assert.Equal(new CoreCommand.MeetingDiscard(), rig.Sent.Commands[^1]);
        Assert.Equal(new DropLine(MeetingDrop.DiscardingTitle, MeetingDrop.DiscardingLine), rig.Line);
        rig.Apply("""{"type":"meeting.stopped","record":"r1"}""");
        Assert.Equal(MeetingDrop.DiscardingLine, rig.Line.Detail); // not the final pass: none runs
        rig.Apply("""{"type":"meeting.discarded","record":"r1","audio_left":false,"scrubbed":true}""");
        Assert.Null(rig.Store.Meeting);
        Assert.Null(rig.Store.LastRecord);
        Assert.Null(rig.Drop.Line);
        Assert.Null(rig.Screens.Meetings.Discarding);
    }

    [Fact]
    public void AStopAndDeleteRefusedAfterTheMinuteLeavesOnlyStop()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, InAMinute));
        rig.Screens.Meetings.Discard();
        rig.Apply("""{"type":"command.failed","command":"meeting.discard","id":"meeting.discard","code":"delete_window_over","message":"the first minute is over: stop the meeting, then delete it from the library"}""");
        Assert.Equal(MeetingModel.DeleteWindowOverText, rig.Line.Detail);
        Assert.Equal(DropLineTone.Alert, rig.Line.Tone);
        Assert.Equal(["Stop"], rig.Titles);
        var before = rig.Sent.Commands.Count;
        rig.Screens.Meetings.Discard();
        Assert.Equal(before, rig.Sent.Commands.Count); // not offered, not sent
        // The transcript moving on takes the refusal's words away; the next offer never shows them.
        rig.Apply("""{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"next item"}""");
        Assert.Equal("next item", rig.Line.Detail);
        rig.Apply("""{"type":"meeting.finished","record":"r1","revision":2}""", Offered);
        Assert.Equal(MeetingDrop.ConsentLine, rig.Line.Detail);
    }

    [Fact]
    public void OtherDiscardRefusalsKeepTheButtonAndLateOnesShowNowhere()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, InAMinute));
        rig.Screens.Meetings.Discard();
        rig.Apply("""{"type":"command.failed","command":"meeting.discard","id":"meeting.discard","message":"database is locked"}""");
        Assert.Equal("Couldn't delete it: database is locked", rig.Line.Detail);
        Assert.Equal(["Stop", "Stop and delete"], rig.Titles); // pressed again, it may pass
        rig.Screens.Meetings.Discard();
        rig.Apply("""{"type":"meeting.failed","record":"r1","message":"capture stopped"}""");
        Assert.Null(rig.Screens.Meetings.Discarding);
        rig.Apply("""{"type":"command.failed","command":"meeting.discard","id":"meeting.discard","message":"the meeting had already stopped, or failed"}""");
        Assert.Null(rig.Screens.Meetings.FailureOn(MeetingPlace.Drop));
        Assert.Contains(rig.Logged.Messages, m => m.Contains("nothing shows it", StringComparison.Ordinal));
        rig.Apply(Started(auto: true, InAMinute, "r2"));
        rig.Screens.Meetings.Discard();
        rig.Apply("""{"type":"core.stopped"}""");
        Assert.Null(rig.Screens.Meetings.Discarding);
        Assert.False(rig.Screens.Meetings.CanDiscard("r2"));
    }

    /// <summary>The far end going silent in an automatic call keeps its Stop and Stop and delete.</summary>
    [Fact]
    public void TheWarningKeepsAnAutoCallsStop()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, InAMinute), """{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}""");
        Assert.Equal(DropInk.Problem, rig.Drop.Ink);
        Assert.Equal(["Stop", "Stop and delete"], rig.Titles);
    }

    /// <summary>A failed Stop stays said while its call records; an earlier meeting's end never takes the live one's.</summary>
    [Fact]
    public void DropFailuresLastAsLongAsTheyMean()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, InAMinute));
        rig.Screens.PerformDropAction(new DropAction.StopRecording());
        rig.Apply("""{"type":"command.failed","command":"meeting.stop","id":"meeting.stop","message":"the capture did not answer"}""");
        Assert.Equal("Couldn't stop: the capture did not answer", rig.Line.Detail);
        rig.Apply("""{"type":"meeting.final","record":"r1","channel":"far","start_ms":0,"end_ms":900,"text":"still talking"}""");
        Assert.Equal("Couldn't stop: the capture did not answer", rig.Line.Detail);
        rig.Screens.Meetings.Apply(Ev.Of("""{"type":"meeting.finished","record":"r0","revision":2}"""));
        Assert.NotNull(rig.Screens.Meetings.FailureOn(MeetingPlace.Drop));
        rig.Apply("""{"type":"meeting.finished","record":"r1","revision":2}""");
        Assert.Null(rig.Screens.Meetings.FailureOn(MeetingPlace.Drop));
    }

    /// <summary>"Always for": recorded only once saved and while still offered; a failed save records nothing and is said on the offer.</summary>
    [Fact]
    public void AlwaysForRecordsOnlyOnceItIsSaved()
    {
        var rig = new Rig();
        rig.Apply(Offered);
        rig.Screens.PerformDropAction(new DropAction.Always(Zoom, "Zoom"));
        rig.Apply($$"""{"type":"command.failed","command":"meetings.calls.set","id":"{{rig.LastSetRef}}","message":"database is locked"}""");
        Assert.DoesNotContain(new CoreCommand.MeetingStart(Zoom, null), rig.Sent.Commands);
        Assert.Equal("Couldn't save that: database is locked", rig.Line.Detail);
        Assert.Null(rig.Screens.Calls.Failure); // said where it was asked

        // Recorded by hand after all: the failure goes with the offer, never on a later call's Drop.
        rig.Apply(Started(auto: false, null));
        Assert.Null(rig.Screens.Calls.DropFailure);
        rig.Apply("""{"type":"meeting.finished","record":"r1","revision":2}""", Started(auto: true, null, "r2"));
        Assert.Equal(MeetingDrop.AutoReminder("Zoom"), rig.Line.Detail);

        // The call ended before Always was saved: nothing is recorded for it.
        rig.Apply("""{"type":"meeting.finished","record":"r2","revision":2}""", Offered);
        rig.Screens.PerformDropAction(new DropAction.Always(Zoom, "Zoom"));
        var set = rig.LastSetRef;
        rig.Apply("""{"type":"meeting.detection_ended","app":"zoom.exe","dismissed":false}""");
        var sentBefore = rig.Sent.Commands.Count;
        rig.Apply(Calls("""{"app":"zoom.exe","app_name":"zoom","policy":"always","chosen":true}""", @ref: set));
        Assert.Equal(sentBefore, rig.Sent.Commands.Count); // saved, not recorded

        // Over a list the core can't read, nothing is sent from the Drop.
        rig.Apply(Offered, Calls("", message: "the stored choices cannot be read"));
        sentBefore = rig.Sent.Commands.Count;
        rig.Screens.PerformDropAction(new DropAction.Always(Zoom, "Zoom"));
        Assert.Equal(sentBefore, rig.Sent.Commands.Count);
        Assert.Equal(CallPolicyModel.UnreadableFromDrop, rig.Line.Detail);
    }

    /// <summary>Notes typed in a meeting being stopped and deleted are never saved into it; a delete refused after the stop saves them.</summary>
    [Fact]
    public void NotesAreNotSavedIntoAMeetingBeingDeleted()
    {
        var rig = new Rig();
        rig.Apply(Started(auto: true, InAMinute));
        rig.Screens.Live.NotesEdited("a thought of my own", caretParagraph: 0);
        rig.Screens.Meetings.Discard();
        rig.Apply("""{"type":"meeting.stopped","record":"r1"}""", """{"type":"meeting.discarded","record":"r1","audio_left":false,"scrubbed":true}""");
        Assert.DoesNotContain(rig.Sent.Commands, c => c is CoreCommand.NoteAdd);

        rig.Apply(Started(auto: true, InAMinute, "r2"));
        rig.Screens.Live.NotesEdited("kept after all", caretParagraph: 0);
        rig.Screens.Meetings.Discard();
        rig.Apply("""{"type":"meeting.stopped","record":"r2"}""");
        Assert.DoesNotContain(rig.Sent.Commands, c => c is CoreCommand.NoteAdd); // held while the delete is pending
        rig.Apply("""{"type":"command.failed","command":"meeting.discard","id":"meeting.discard","message":"the meeting had already stopped, or failed"}""");
        Assert.Contains(rig.Sent.Commands, c => c is CoreCommand.NoteAdd); // refused: saved
    }

    [Fact]
    public void TheScreensShowTheNewFailures()
    {
        var screens = new ScreenModels(new Sent().Send, log: new Logged().Log);
        foreach (var command in new[] { "meeting.discard", "meetings.calls.list", "meetings.calls.set" })
        {
            Assert.True(screens.Handles(Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"{{command}}","message":"x"}""")), command);
        }
        Assert.True(screens.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:meetings.calls.default","message":"x"}""")));
    }
}
