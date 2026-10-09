// What the Drop says about dictation, and when (the Mac's DropText.dictating, DictationModel.note
// and DropController's order): a held take's line over its live words, then a note for a few
// seconds when a take did not go in as it should, a note during a take kept for its end. Every
// word here is synthetic.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public sealed class DropModelTests
{
    /// <summary>Wakes the test runs by hand.</summary>
    private sealed class FakeWakes : IWakeScheduler
    {
        public List<(TimeSpan Delay, Action Wake, Pending Handle)> Scheduled { get; } = [];

        public IDisposable After(TimeSpan delay, Action wake)
        {
            var handle = new Pending();
            Scheduled.Add((delay, wake, handle));
            return handle;
        }

        /// <summary>Runs every wake not cancelled, as the dispatcher would when their time comes.</summary>
        public void RunDue()
        {
            foreach (var (_, wake, handle) in Scheduled.ToList())
            {
                if (!handle.Cancelled)
                {
                    handle.Dispose();
                    wake();
                }
            }
        }

        public int Waiting => Scheduled.Count(s => !s.Handle.Cancelled);

        public sealed class Pending : IDisposable
        {
            public bool Cancelled { get; private set; }

            public void Dispose() => Cancelled = true;
        }
    }

    private sealed class Rig
    {
        public CoreStore Store { get; } = new();
        public FakeWakes Wakes { get; } = new();
        public DropModel Drop { get; }
        public int Changes { get; private set; }

        public Rig(bool hasLanguageModel = false, bool noSpeechModel = false)
        {
            Drop = new DropModel(Wakes, () => hasLanguageModel, noSpeechModel: () => noSpeechModel);
            Drop.Changed += () => Changes++;
        }

        public void Apply(params string[] events)
        {
            var batch = events.Select(Ev.Of).ToList();
            Store.Apply(batch);
            Drop.Apply(Store, batch);
        }
    }

    private const string Started = """{"type":"dictation.started","take":0,"edit":false,"mode":"Chat","app":"Notepad"}""";
    private const string Stopped = """{"type":"dictation.stopped"}""";

    [Fact]
    public void ATakeShowsTheAppItsModeAndItsLiveWordsThenHides()
    {
        var rig = new Rig();
        Assert.Null(rig.Drop.Line);
        rig.Apply(Started);
        Assert.True(rig.Drop.IsLive);
        Assert.Equal(new DropLine("Dictating · Notepad · Chat", "Listening"), rig.Drop.Line);

        rig.Apply("""{"type":"dictation.partial","take":0,"text":"a synthetic line"}""");
        Assert.Equal(new DropLine("Dictating · Notepad · Chat", "a synthetic line", LiveWords: true), rig.Drop.Line);

        rig.Apply(Stopped);
        Assert.Equal(new DropLine("Dictating · Notepad · Chat", "Transcribing"), rig.Drop.Line);

        // Typed as it should be: nothing to say, the Drop hides at once.
        rig.Apply("""{"type":"dictation.inserted","text":"A synthetic line.","outcome":"pasted"}""");
        Assert.Null(rig.Drop.Line);
        Assert.False(rig.Drop.IsLive);
        Assert.Equal(0, rig.Wakes.Waiting);
    }

    [Fact]
    public void ATakeWithNoAppOrModeSaysDictating()
    {
        var rig = new Rig();
        rig.Apply("""{"type":"dictation.started","take":0,"edit":false}""");
        Assert.Equal(new DropLine("Dictating", "Listening"), rig.Drop.Line);
        rig.Apply("""{"type":"dictation.partial","take":0,"text":"  "}""");
        Assert.Equal(new DropLine("Dictating", "Listening"), rig.Drop.Line);
    }

    [Fact]
    public void AVoiceEditSaysWhatItDoes()
    {
        var rig = new Rig();
        rig.Apply("""{"type":"dictation.started","take":0,"edit":true}""");
        Assert.Equal(new DropLine("Editing the selection", "Say what to change"), rig.Drop.Line);
        rig.Apply(Stopped);
        Assert.Equal(new DropLine("Editing the selection", "Rewriting"), rig.Drop.Line);
    }

    [Fact]
    public void ATooShortTakeShowsItsNoteForAFewSecondsWithTheInkStill()
    {
        var rig = new Rig();
        rig.Apply(Started, Stopped, """{"type":"dictation.discarded","reason":"speech_too_short"}""");
        Assert.Equal(new DropLine("Too short", "Try again"), rig.Drop.Line);
        Assert.False(rig.Drop.IsLive);
        var (delay, _, _) = Assert.Single(rig.Wakes.Scheduled);
        Assert.Equal(DropModel.NoteDuration, delay);

        rig.Wakes.RunDue();
        Assert.Null(rig.Drop.Line);
    }

    [Fact]
    public void ANoteDuringATakeWaitsForItsEnd()
    {
        var rig = new Rig();
        rig.Apply(Started);
        // Windows removed the edit key's hook mid-take (the dictation key still holds): said when
        // the take is over.
        rig.Apply("""{"type":"dictation.edit_hotkey_lost"}""");
        Assert.Equal("Listening", rig.Drop.Line!.Detail);
        Assert.True(rig.Drop.IsLive);
        rig.Apply(Stopped, """{"type":"dictation.inserted","text":"x","outcome":"typed"}""");
        Assert.Equal(new DropLine("The edit key stopped working", "Turn dictation on again in Settings", DropLineTone.Alert), rig.Drop.Line);
        Assert.False(rig.Drop.IsLive);
    }

    [Fact]
    public void ALostDictationKeyEndsTheTakeAndSaysSoAtOnce()
    {
        var rig = new Rig();
        rig.Apply(Started);
        rig.Apply("""{"type":"dictation.hotkey_lost"}""");
        Assert.Equal(new DropLine("The dictation key stopped working", "Turn dictation on again in Settings", DropLineTone.Alert), rig.Drop.Line);
        Assert.False(rig.Drop.IsLive);
    }

    [Fact]
    public void ANewTakeEndsANoteAndItsTimerIsHarmless()
    {
        var rig = new Rig();
        rig.Apply(Started, Stopped, """{"type":"dictation.discarded","reason":"too_short"}""");
        Assert.Equal("Too short", rig.Drop.Line!.Title);
        rig.Apply("""{"type":"dictation.started","take":1,"edit":false}""");
        Assert.True(rig.Drop.IsLive);
        Assert.Equal(0, rig.Wakes.Waiting);
        var changes = rig.Changes;
        // The first note's wake, had it run anyway, changes nothing.
        rig.Wakes.Scheduled[0].Wake();
        Assert.Equal(changes, rig.Changes);
        Assert.True(rig.Drop.IsLive);
    }

    [Fact]
    public void AnAppRunningAsAdministratorIsSaidInWindowsWords()
    {
        var rig = new Rig();
        rig.Apply(Started, Stopped, """{"type":"dictation.inserted","text":"x","outcome":"blocked"}""");
        Assert.Equal(new DropLine("Can't type into this app", "It runs as administrator; your words are in the Library", DropLineTone.Alert), rig.Drop.Line);
        Assert.Equal(DropLineTone.Alert, DictationDrop.Note(Ev.Of("""{"type":"dictation.edit_failed","reason":"secure_input"}"""), false)!.Tone);
    }

    /// <summary>
    /// A hold with no speech model installed says so, not "the microphone is silent" nor "couldn't
    /// transcribe that"; with a model the usual lines come back.
    /// </summary>
    [Fact]
    public void AHoldWithNoSpeechModelSaysThereIsNone()
    {
        var none = new DropLine("No speech model is installed", "Settings > Models downloads one", DropLineTone.Alert);
        var rig = new Rig(noSpeechModel: true);
        rig.Apply(Started, Stopped, """{"type":"dictation.discarded","reason":"silence"}""");
        Assert.Equal(none, rig.Drop.Line);
        var failed = new Rig(noSpeechModel: true);
        failed.Apply(Started, Stopped, """{"type":"dictation.failed","stage":"transcription","message":"model not installed"}""");
        Assert.Equal(none, failed.Drop.Line);
        var withModel = new Rig();
        withModel.Apply(Started, Stopped, """{"type":"dictation.discarded","reason":"silence"}""");
        Assert.Equal("The microphone is silent", withModel.Drop.Line!.Title);
    }

    [Fact]
    public void EachNoteSaysWhatTheMacSaysWhereTheCauseIsTheSame()
    {
        DropLine? Note(string json, bool model = false) => DictationDrop.Note(Ev.Of(json), model);
        Assert.Equal(new DropLine("No speech heard", "Nothing was typed"), Note("""{"type":"dictation.discarded","reason":"no_speech"}"""));
        Assert.Equal(DropLineTone.Alert, Note("""{"type":"dictation.discarded","reason":"silence"}""")!.Tone);
        Assert.Null(Note("""{"type":"dictation.discarded","reason":"cancelled"}"""));
        Assert.Equal("Couldn't transcribe that", Note("""{"type":"dictation.failed","stage":"transcription","message":"m"}""")!.Title);
        Assert.Equal("Couldn't type it here", Note("""{"type":"dictation.failed","stage":"insert","message":"m"}""")!.Title);
        Assert.Equal(new DropLine("Typed", "Your clipboard couldn't be put back"),
            Note("""{"type":"dictation.inserted","text":"x","outcome":"inserted_clipboard_not_restored"}"""));
        Assert.Null(Note("""{"type":"dictation.inserted","text":"x","outcome":"pasted"}"""));
        Assert.Equal("Couldn't open the microphone", Note("""{"type":"dictation.mic_failed","message":"no input device"}""")!.Title);
        Assert.Equal("Select some text first", Note("""{"type":"dictation.edit_failed","reason":"no_selection"}""")!.Title);
        Assert.Equal("Editing needs a language model", Note("""{"type":"dictation.edit_failed","reason":"no_model"}""")!.Title);
        Assert.Equal("Couldn't rewrite the selection", Note("""{"type":"dictation.edit_failed","reason":"model"}""", model: true)!.Title);
        Assert.Equal("Not edited", Note("""{"type":"dictation.edit_failed","reason":"not_allowed","message":"Example Cloud"}""")!.Title);
        Assert.Equal("Polish took too long", Note("""{"type":"dictation.warning","kind":"polish_timed_out"}""")!.Title);
        // The mode names a model of its own the core does not hold, or that sends elsewhere now:
        // nothing was sent, and Settings > Modes says which (it was quiet before).
        Assert.Equal(new DropLine("Not polished", "Check this mode's model in Settings > Modes", DropLineTone.Alert),
            Note("""{"type":"dictation.warning","kind":"polish_model_missing","message":"the mode's model is not held"}"""));
        Assert.Equal("Stopped after 3 minutes", Note("""{"type":"dictation.warning","kind":"release_missed"}""")!.Title);
        Assert.Null(Note("""{"type":"dictation.warning","kind":"focus_unreadable"}"""));
    }

    [Fact]
    public void ChangesAreSignalledOncePerBatchAndOnlyWhenSomethingChanged()
    {
        var rig = new Rig();
        rig.Apply("""{"type":"setting.value","key":"dictation.key","value":"right_alt"}""");
        Assert.Equal(0, rig.Changes);
        rig.Apply(Started, """{"type":"dictation.partial","take":0,"text":"one"}""");
        Assert.Equal(1, rig.Changes);
        rig.Apply("""{"type":"dictation.partial","take":0,"text":"one"}""");
        Assert.Equal(1, rig.Changes);
    }
}
