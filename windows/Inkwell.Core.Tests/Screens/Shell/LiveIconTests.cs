// The live icon's looks and redraws, as the Mac's LiveIconLookTests, LiveIconPulseTests and
// LiveIconAsleepTests: what each state shows, the final pass's progress, a pulse that runs only
// while recording and someone can see the screen, and the frames each state costs.
using Inkwell.Core.Events;
using Inkwell.Core.Glow;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class LiveIconTests
{
    private sealed class RecordingSurface : ILiveIconSurface
    {
        public List<LiveIconFrame> Shown { get; } = [];

        public void Show(LiveIconFrame frame) => Shown.Add(frame);
    }

    /// <summary>A ticker the test fires by hand; a late tick fires the stopped timer's closure.</summary>
    private sealed class HandTicker : ILiveIconTicker
    {
        private Action? tick;
        private Action? last;

        public List<TimeSpan> Starts { get; } = [];
        public int Stops { get; private set; }
        public bool Running => tick is not null;

        public void Start(TimeSpan interval, Action tick)
        {
            Starts.Add(interval);
            this.tick = tick;
        }

        public void Cancel()
        {
            if (tick is not null)
            {
                Stops++;
            }
            last = tick;
            tick = null;
        }

        public void Fire(int times = 1)
        {
            for (var i = 0; i < times; i++)
            {
                tick?.Invoke();
            }
        }

        public void FireLate(int times = 1)
        {
            for (var i = 0; i < times; i++)
            {
                (tick ?? last)?.Invoke();
            }
        }
    }

    private static readonly LiveIconColours Colours = new(new GlowRgb(0.4, 0.3, 1), new GlowRgb(1, 0.6, 0.3), new GlowRgb(0.9, 0.3, 0.2));

    private static LiveIconLook Look(DropInk state, double? progress = null, bool still = false) => LiveIconLook.For(state, progress, still);

    [Fact]
    public void EachStateHasItsLook()
    {
        Assert.Equal(new LiveIconLook.Rest(), Look(DropInk.Idle));
        Assert.Equal(new LiveIconLook.Glow(LiveIconTone.You), Look(DropInk.Dictating));
        Assert.Equal(new LiveIconLook.Pulse(LiveIconTone.Them), Look(DropInk.Meeting));
        Assert.Equal(new LiveIconLook.Ring(0.5), Look(DropInk.Blotting, 0.5));
        Assert.Equal(new LiveIconLook.Ring(null), Look(DropInk.Blotting)); // no number: indeterminate
        Assert.Equal(new LiveIconLook.Glow(LiveIconTone.Alert), Look(DropInk.Problem));
    }

    /// <summary>Always still and animations off: the recording is a still frame in their colour; nothing else moved anyway.</summary>
    [Fact]
    public void StillHoldsTheRecordingStill()
    {
        Assert.Equal(new LiveIconLook.Glow(LiveIconTone.Them), Look(DropInk.Meeting, still: true));
        Assert.All(Enum.GetValues<DropInk>(), s => Assert.False(Look(s, 0.25, still: true).Pulses));
        Assert.Equal([DropInk.Meeting], Enum.GetValues<DropInk>().Where(s => Look(s).Pulses));
    }

    /// <summary>The ring fills with the steps the final pass has reported, out of the four it can report; before the first, no number.</summary>
    [Fact]
    public void TheFinalPassProgressIsTheStepsReported()
    {
        var meeting = new LiveMeeting("r1");
        Assert.Null(LiveIcon.FinalPassProgress(null));
        Assert.Null(LiveIcon.FinalPassProgress(meeting));
        meeting = meeting with { Blotted = meeting.Blotted with { Transcribed = true, MicTranscribed = true } };
        Assert.Equal(0.25, LiveIcon.FinalPassProgress(meeting));
        meeting = meeting with { Blotted = meeting.Blotted with { FarTranscribed = true } };
        Assert.Equal(0.5, LiveIcon.FinalPassProgress(meeting));
        meeting = meeting with { Blotted = meeting.Blotted with { Diarized = true } };
        Assert.Equal(0.75, LiveIcon.FinalPassProgress(meeting));
        meeting = meeting with { Blotted = meeting.Blotted with { Summarized = true } };
        Assert.Equal(1, LiveIcon.FinalPassProgress(meeting));
        // A later step comes after both sides, as Today reads it.
        var skipped = new LiveMeeting("r2") { Blotted = new BlotProgress(false, false, true) };
        Assert.Equal(0.75, LiveIcon.FinalPassProgress(skipped));
    }

    /// <summary>The store counts each side's transcription once, from the core's events.</summary>
    [Fact]
    public void TheStoreCountsEachSideTranscribed()
    {
        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"meeting.started","record":"m1","started_at_unix_ms":1}""")]);
        Assert.Equal(0, store.Meeting?.Blotted.SidesTranscribed);
        var pass = """{"audible_ms":1,"backlogged_finals":0,"captured_ms":1,"chunks":1,"empty_regions":0,"failed_regions":0,"regions":1,"speech_ms":1,"word_count":1}""";
        store.Apply([Ev.Of($$"""{"type":"meeting.transcribed","record":"m1","pass":{{pass.Replace("{", "{\"channel\":\"mic\",", StringComparison.Ordinal)}}}""")]);
        store.Apply([Ev.Of($$"""{"type":"meeting.transcribed","record":"m1","pass":{{pass.Replace("{", "{\"channel\":\"mic\",", StringComparison.Ordinal)}}}""")]);
        Assert.Equal(1, store.Meeting?.Blotted.SidesTranscribed);
        store.Apply([Ev.Of($$"""{"type":"meeting.transcribed","record":"m1","pass":{{pass.Replace("{", "{\"channel\":\"far\",", StringComparison.Ordinal)}}}""")]);
        Assert.Equal(2, store.Meeting?.Blotted.SidesTranscribed);
        Assert.True(store.Meeting?.Blotted.Transcribed);
    }

    [Fact]
    public void NoTimerAtRestAndNothingRedrawsAfterSettling()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        Assert.Equal([new LiveIconLook.Rest()], surface.Shown.Select(f => f.Look)); // an attached surface is told once what to show
        icon.Update(new LiveIconLook.Rest(), Colours);
        icon.Update(new LiveIconLook.Rest(), Colours);
        Assert.Single(surface.Shown); // the same look again draws nothing
        Assert.Empty(ticker.Starts);
        Assert.False(icon.IsPulsing);
    }

    /// <summary>Dictating is a still frame: drawn when the state or a colour changes, and never between.</summary>
    [Fact]
    public void AStillLookRedrawsOnlyOnChange()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        icon.Update(new LiveIconLook.Glow(LiveIconTone.You), Colours);
        icon.Update(new LiveIconLook.Glow(LiveIconTone.You), Colours);
        Assert.Equal(2, surface.Shown.Count);
        icon.Update(new LiveIconLook.Glow(LiveIconTone.You), Colours with { You = new GlowRgb(0, 1, 0) });
        Assert.Equal(3, surface.Shown.Count); // a new colour is one frame
        Assert.Equal(1, surface.Shown[^1].Strength);
        Assert.Empty(ticker.Starts); // no timer for a still look
    }

    [Fact]
    public void ThePulseStartsAndStopsWithTheRecording()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        icon.Update(new LiveIconLook.Pulse(LiveIconTone.Them), Colours);
        Assert.True(icon.IsPulsing);
        Assert.Equal(LiveIcon.PulseFps, 1 / Assert.Single(ticker.Starts).TotalSeconds, 3);
        Assert.Equal(1, surface.Shown[^1].Strength); // the recording shows at once, at full strength

        // The same look again (a colour or progress echo) does not restart the cycle.
        icon.Update(new LiveIconLook.Pulse(LiveIconTone.Them), Colours);
        Assert.Single(ticker.Starts);

        var before = surface.Shown.Count;
        ticker.Fire((int)(LiveIcon.PulseFps * LiveIcon.BreathPeriod));
        Assert.Equal(before + 14, surface.Shown.Count); // one frame a tick: 7 fps
        var strengths = surface.Shown.TakeLast(14).Select(f => f.Strength).ToList();
        Assert.True(strengths.Min() < 0.7, "it breathes out");
        Assert.Equal(1, strengths[^1], 3); // and back in, every two seconds

        icon.Update(new LiveIconLook.Ring(null), Colours);
        Assert.False(icon.IsPulsing); // stopped the moment recording ends
        Assert.Equal(1, ticker.Stops);
        Assert.Equal(new LiveIconFrame(new LiveIconLook.Ring(null), Colours, 1), surface.Shown[^1]);
    }

    /// <summary>A tick already on its way when the recording ended draws nothing.</summary>
    [Fact]
    public void ALateTickDrawsNothing()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        icon.Update(new LiveIconLook.Pulse(LiveIconTone.Them), Colours);
        icon.Update(new LiveIconLook.Glow(LiveIconTone.You), Colours);
        var shown = surface.Shown.Count;
        ticker.FireLate(3);
        Assert.Equal(shown, surface.Shown.Count);
    }

    [Fact]
    public void StillMeansNoTimer()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        icon.Attach(new RecordingSurface());
        icon.Update(Look(DropInk.Meeting, still: true), Colours);
        Assert.False(icon.IsPulsing);
        Assert.Empty(ticker.Starts);
    }

    /// <summary>
    /// Windows' shell icons change only with the state: each of their frames is a call into
    /// Explorer, so a recording is one still frame in their colour and no timer runs.
    /// </summary>
    [Fact]
    public void TheShellsIconsChangeOnlyWithTheState()
    {
        Assert.All(Enum.GetValues<DropInk>(), state => Assert.False(LiveIconLook.OnShell(state, null).Pulses));
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        icon.Update(LiveIconLook.OnShell(DropInk.Meeting, null), Colours);
        icon.Update(LiveIconLook.OnShell(DropInk.Meeting, null), Colours);
        ticker.Fire((int)(LiveIcon.PulseFps * LiveIcon.BreathPeriod));
        Assert.Empty(ticker.Starts);
        Assert.Equal([new LiveIconLook.Rest(), new LiveIconLook.Glow(LiveIconTone.Them)], surface.Shown.Select(f => f.Look));
    }

    /// <summary>With nothing to draw on, nothing ticks; the pulse resumes on the next surface.</summary>
    [Fact]
    public void NoSurfaceMeansNoTimer()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        icon.Update(new LiveIconLook.Pulse(LiveIconTone.Them), Colours);
        Assert.False(icon.IsPulsing);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        Assert.True(icon.IsPulsing);
        icon.Detach(surface);
        Assert.False(icon.IsPulsing);
        ticker.FireLate();
        Assert.Single(surface.Shown); // a detached surface is drawn no more
    }

    /// <summary>Frames per minute, on a simulated clock: 0 at rest, 1 for a dictation that starts, 420 for a minute's recording (7 fps).</summary>
    [Fact]
    public void FramesPerMinute()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        icon.Attach(new RecordingSurface());
        var minute = (int)(LiveIcon.PulseFps * 60);

        var start = icon.Frames;
        icon.Update(new LiveIconLook.Rest(), Colours);
        ticker.Fire(minute);
        Assert.Equal(0, icon.Frames - start); // idle

        start = icon.Frames;
        for (var i = 0; i <= minute; i++)
        {
            icon.Update(new LiveIconLook.Glow(LiveIconTone.You), Colours);
        }
        ticker.Fire(minute);
        Assert.Equal(1, icon.Frames - start); // dictating: the one change

        icon.Update(new LiveIconLook.Pulse(LiveIconTone.Them), Colours);
        start = icon.Frames;
        ticker.Fire(minute);
        Assert.Equal(420, icon.Frames - start); // recording
    }

    /// <summary>Nobody can see the screen (locked, or the display off): nothing ticks or draws; awake again, the state as it is now, once.</summary>
    [Fact]
    public void AsleepNothingTicksOrDraws()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        icon.Update(new LiveIconLook.Pulse(LiveIconTone.Them), Colours);
        Assert.True(icon.IsPulsing);

        icon.SetAwake(false);
        Assert.False(icon.IsPulsing); // the timer stops
        var start = icon.Frames;
        var shown = surface.Shown.Count;
        ticker.FireLate(420);
        icon.Update(new LiveIconLook.Ring(0.25), Colours);
        icon.Update(new LiveIconLook.Ring(0.5), Colours);
        Assert.Equal(0, icon.Frames - start); // a minute asleep, with the state changing: nothing drawn
        Assert.Equal(shown, surface.Shown.Count);

        icon.SetAwake(true);
        Assert.Equal(new LiveIconLook.Ring(0.5), surface.Shown[^1].Look); // awake: the state as it is now, at once
        Assert.Equal(1, icon.Frames - start);
        Assert.False(icon.IsPulsing); // the recording ended while asleep
    }

    /// <summary>Awake again mid-recording: the breath picks up, with nothing to redraw.</summary>
    [Fact]
    public void TheBreathResumesOnWake()
    {
        var ticker = new HandTicker();
        var icon = new LiveIcon(ticker);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        icon.Update(new LiveIconLook.Pulse(LiveIconTone.Them), Colours);
        icon.SetAwake(false);
        icon.SetAwake(false);
        var shown = surface.Shown.Count;
        icon.SetAwake(true);
        Assert.True(icon.IsPulsing);
        Assert.Equal(2, ticker.Starts.Count);
        Assert.Equal(shown, surface.Shown.Count); // nothing changed while asleep
        ticker.Fire();
        Assert.Equal(shown + 1, surface.Shown.Count);
    }

    /// <summary>A surface attached while nobody can see is shown nothing until they can.</summary>
    [Fact]
    public void ASurfaceAttachedAsleepIsShownOnWaking()
    {
        var icon = new LiveIcon(new HandTicker());
        icon.Update(new LiveIconLook.Pulse(LiveIconTone.Them), Colours);
        icon.SetAwake(false);
        var surface = new RecordingSurface();
        icon.Attach(surface);
        Assert.Empty(surface.Shown); // nothing drawn on a sleeping display
        Assert.False(icon.IsPulsing);
        icon.SetAwake(true);
        Assert.Equal([new LiveIconLook.Pulse(LiveIconTone.Them)], surface.Shown.Select(f => f.Look)); // shown on waking
    }

    /// <summary>
    /// A locked session, or one disconnected from its screen (another user switched to, a remote
    /// desktop closed), holds the icon asleep until its own unlock or connect: the app coming to the
    /// front meanwhile ends neither.
    /// </summary>
    [Fact]
    public void ALockOrADisconnectHoldsUntilItsOwnEnd()
    {
        var viewers = new LiveIconViewers();
        Assert.True(viewers.CanSee);
        viewers.Connect(false);
        Assert.False(viewers.CanSee);
        viewers.AppActive();
        Assert.False(viewers.CanSee); // brought forward while nobody is there
        viewers.Connect(true);
        Assert.True(viewers.CanSee);

        viewers.Lock(true);
        viewers.AppActive();
        Assert.False(viewers.CanSee);
        viewers.Connect(false);
        viewers.Connect(true);
        Assert.False(viewers.CanSee); // connected again, and still locked
        viewers.Lock(false);
        Assert.True(viewers.CanSee);
    }

    /// <summary>The display off holds it until the display is on, or the app comes to the front (someone pressed something).</summary>
    [Fact]
    public void TheDisplayOffHoldsUntilItIsOnOrTheAppComesForward()
    {
        var viewers = new LiveIconViewers();
        viewers.Display(false);
        Assert.False(viewers.CanSee);
        viewers.Display(true);
        Assert.True(viewers.CanSee);
        viewers.Display(false);
        viewers.AppActive();
        Assert.True(viewers.CanSee);
    }

    /// <summary>Narrator hears the state: the tray's name and the taskbar overlay's description.</summary>
    [Fact]
    public void TheSpokenLabelSaysTheState()
    {
        Assert.Equal("Inkwell", LiveIcon.Spoken(DropInk.Idle));
        Assert.Equal("Inkwell, dictating", LiveIcon.Spoken(DropInk.Dictating));
        Assert.Equal("Inkwell, recording", LiveIcon.Spoken(DropInk.Meeting));
        Assert.Equal("Inkwell, finishing a recording", LiveIcon.Spoken(DropInk.Blotting));
        Assert.Equal("Inkwell, recording; the other side is quiet", LiveIcon.Spoken(DropInk.Problem));
        Assert.Null(LiveIcon.OverlayText(DropInk.Idle));
        Assert.Equal("Recording", LiveIcon.OverlayText(DropInk.Meeting));
        Assert.Equal(("Record", "Stop"), (LiveIcon.RecordButton, LiveIcon.StopButton));
    }

    [Fact]
    public void TheBreathGoesOutAndBack()
    {
        Assert.Equal(1, LiveIcon.Breath(0), 6);
        Assert.Equal(LiveIcon.BreathLow, LiveIcon.Breath(7), 6); // half a breath: the faintest
        Assert.Equal(1, LiveIcon.Breath(14), 6);
    }
}
