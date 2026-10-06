// Settings > Sound (as the Mac's SoundTests): the microphone picker (Automatic first, then each mic
// with how it connects, and a chosen mic that isn't connected), "Their sound from" (Windows' output
// picker), their captions, and the mic test with its meter. Every device name here is synthetic.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public sealed class SoundModelTests
{
    private const string Laptop = """{"id":"mic","name":"Microphone (Realtek Audio)","transport":"built_in","reason":"default_input"}""";

    /// <summary>An audio.devices answer: the PC's mic, a headset and a loopback, the speakers and a dock.</summary>
    internal static InkEvent Devices(
        string input = "auto", string usingMic = Laptop, string? wanted = null, string type = "audio.devices",
        string output = "default", string? outputWanted = null,
        string outputUsing = """{"id":"spk","name":"Speakers (Realtek Audio)","transport":"built_in","reason":"default_output"}""")
    {
        var wantedField = wanted is null ? "" : $$""","wanted":{{wanted}}""";
        var outputWantedField = outputWanted is null ? "" : $$""","output_wanted":{{outputWanted}}""";
        var refField = type == "audio.devices" ? ""","ref":"sound.devices" """ : "";
        return Ev.Of($$"""
            {"type":"{{type}}"{{refField}},"input":"{{input}}"{{wantedField}},
             "inputs":[{"id":"mic","name":"Microphone (Realtek Audio)","transport":"built_in","is_default":true},
                       {"id":"hs","name":"Headset (Buds)","transport":"bluetooth","is_default":false},
                       {"id":"odd","name":"Line In","transport":"other","is_default":false}],
             "automatic":{"id":"mic","name":"Microphone (Realtek Audio)","transport":"built_in","reason":"built_in_for_bluetooth_output"},
             "using":{{usingMic}},
             "outputs":[{"id":"spk","name":"Speakers (Realtek Audio)","transport":"built_in","is_default":true},
                        {"id":"dock","name":"Dock Audio","transport":"usb","is_default":false}],
             "output":"{{output}}"{{outputWantedField}},"output_using":{{outputUsing}}}
            """);
    }

    private static CommandFailed Failed(string command, string? id, string? code = null) =>
        Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"{{command}}"{{(id is null ? "" : $$""","id":"{{id}}" """)}}{{(code is null ? "" : $$""","code":"{{code}}" """)}},"message":"x"}""");

    [Fact]
    public void ThePickersListTheDefaultFirstThenEachDeviceWithHowItConnects()
    {
        var sent = new Sent();
        var sound = new SoundModel(sent.Send);
        sound.Load();
        Assert.Equal([new CoreCommand.AudioDevices(SoundModel.DevicesId)], sent.Commands);
        Assert.Empty(sound.InputChoices);
        Assert.Equal("Reading the microphones…", sound.InputCaption);
        sound.Apply(Devices());
        Assert.Equal(
            ["Automatic (Microphone (Realtek Audio))", "Microphone (Realtek Audio) · Built-in", "Headset (Buds) · Bluetooth", "Line In"],
            sound.InputChoices.Select(c => c.Title));
        Assert.Equal(["auto", "mic", "hs", "odd"], sound.InputChoices.Select(c => c.Id));
        Assert.StartsWith("Automatic follows Windows' input", sound.InputCaption, StringComparison.Ordinal);
        Assert.True(sound.HasOutputs);
        Assert.Equal(
            ["Default output (Speakers (Realtek Audio))", "Speakers (Realtek Audio) · Built-in", "Dock Audio · USB"],
            sound.OutputChoices.Select(c => c.Title));
        Assert.Equal(["default", "spk", "dock"], sound.OutputChoices.Select(c => c.Id));
        Assert.Contains("the default output plays", sound.OutputCaption, StringComparison.Ordinal);
        Assert.Null(sound.MissingLine);
    }

    [Fact]
    public void WithoutOutputsThereIsNoOutputPicker()
    {
        var sound = new SoundModel(new Sent().Send);
        sound.Apply(Ev.Of("""{"type":"audio.devices","input":"auto","inputs":[]}"""));
        Assert.False(sound.HasOutputs);
        Assert.Empty(sound.OutputChoices);
        Assert.Equal(["Automatic"], sound.InputChoices.Select(c => c.Title));
        Assert.Equal("No microphone is connected.", sound.InputCaption);
    }

    [Fact]
    public void ChoosingSendsTheSettingAndARefusalIsSaidUntilThePickAgain()
    {
        var sent = new Sent();
        var sound = new SoundModel(sent.Send);
        sound.Apply(Devices());
        sound.ChooseInput("auto");
        Assert.Empty(sent.Commands); // already the choice
        sound.ChooseInput("hs");
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.AudioInput, "hs"), sent.Commands[^1]);
        Assert.Equal("hs", sound.Current!.Input); // shown at once
        sound.Apply(Failed("setting.set", "setting:audio.input"));
        Assert.Equal("Couldn't choose that microphone: x", sound.InputProblem);
        Assert.Equal(new CoreCommand.AudioDevices(SoundModel.DevicesId), sent.Commands[^1]); // the choice as it really is
        sound.Apply(Devices());
        Assert.Equal("auto", sound.Current!.Input); // snapped back
        Assert.NotNull(sound.InputProblem); // and still says why
        sound.ChooseInput("odd");
        Assert.Null(sound.InputProblem);

        sound.ChooseOutput("dock");
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.AudioOutput, "dock"), sent.Commands[^1]);
        sound.Apply(Failed("setting.set", "setting:audio.output"));
        Assert.Equal("Couldn't choose that output: x", sound.OutputProblem);
        sound.Disappeared();
        Assert.Null(sound.OutputProblem); // leaving Settings clears it

        sound.Apply(Failed("audio.devices", SoundModel.DevicesId));
        Assert.Equal("Couldn't list the devices: x", sound.InputProblem);
        sound.Apply(Devices());
        Assert.Null(sound.InputProblem);
    }

    /// <summary>A chosen device that isn't connected stays chosen, and the captions say who stands in.</summary>
    [Fact]
    public void AChosenDeviceThatIsNotConnectedStaysChosenAndTheCaptionsSayWhoStandsIn()
    {
        var sound = new SoundModel(new Sent().Send);
        sound.Apply(Devices(
            input: "gone",
            usingMic: """{"id":"mic","name":"Microphone (Realtek Audio)","transport":"built_in","reason":"chosen_missing"}""",
            wanted: """{"id":"gone","name":"Studio Mic","transport":"usb"}""",
            output: "tv", outputWanted: """{"id":"tv","name":"TV Audio","transport":"other"}""",
            outputUsing: """{"id":"spk","name":"Speakers (Realtek Audio)","transport":"built_in","reason":"chosen_missing"}"""));
        Assert.Equal(new SoundModel.Choice("gone", "Studio Mic · not connected"), sound.InputChoices[^1]);
        Assert.Equal("Studio Mic isn't connected. Inkwell is using Microphone (Realtek Audio) until it is.", sound.MissingLine);
        Assert.Equal(sound.MissingLine, sound.InputCaption);
        Assert.Equal(new SoundModel.Choice("tv", "TV Audio · not connected"), sound.OutputChoices[^1]);
        Assert.Equal("TV Audio isn't connected. Inkwell records the default output until it is.", sound.OutputCaption);
        // While a choice is on its way, nothing is said about a stand-in.
        sound.ChooseInput("hs");
        Assert.Null(sound.MissingLine);
    }

    [Fact]
    public void AChosenBluetoothMicAndAChosenOutputSayWhatTheyDo()
    {
        var sound = new SoundModel(new Sent().Send);
        sound.Apply(Devices(
            input: "hs", usingMic: """{"id":"hs","name":"Headset (Buds)","transport":"bluetooth","reason":"chosen"}""",
            output: "dock", outputUsing: """{"id":"dock","name":"Dock Audio","transport":"usb","reason":"chosen"}"""));
        Assert.Contains("call quality", sound.InputCaption, StringComparison.Ordinal);
        Assert.Contains("Dock Audio plays, even when Windows' default changes", sound.OutputCaption, StringComparison.Ordinal);
    }

    /// <summary>Test opens the mic; the level follows the core's reports, only while this screen's test runs; Stop asks the core.</summary>
    [Fact]
    public void TheTestRunsItsLevelAndSaysWhetherItHeardYou()
    {
        var sent = new Sent();
        var sound = new SoundModel(sent.Send);
        sound.Load();
        sound.Apply(Devices());
        Assert.Equal("Speak for a few seconds to see the level. Nothing is kept.", sound.TestLine);
        sound.ToggleTest();
        Assert.Equal(new CoreCommand.AudioTest(SoundModel.TestId), sent.Commands[^1]);
        Assert.Equal(SoundModel.TestState.Starting, sound.Test);
        sound.Apply(Ev.Of("""{"type":"audio.test_level","ref":"sound.test","level":0.9}"""));
        Assert.Equal(0, sound.TestLevel); // not started yet
        sound.Apply(Ev.Of("""{"type":"audio.test_started","ref":"other","mic_name":"M","mic_transport":"usb","mic_reason":"chosen","seconds":15}"""));
        Assert.Equal(SoundModel.TestState.Starting, sound.Test); // another's
        sound.Apply(Ev.Of("""{"type":"audio.test_started","ref":"sound.test","mic_name":"Microphone (Realtek Audio)","mic_transport":"built_in","mic_reason":"default_input","seconds":15}"""));
        Assert.Equal("Listening with Microphone (Realtek Audio)…", sound.TestLine);
        sound.Apply(Ev.Of("""{"type":"audio.test_level","ref":"sound.test","level":0.42}"""));
        Assert.Equal(0.42, sound.TestLevel);
        Assert.Equal("42 percent", sound.LevelSpoken);
        sound.Apply(Ev.Of("""{"type":"audio.test_level","ref":"sound.test","level":1.7}"""));
        Assert.Equal(1, sound.TestLevel); // clamped
        sound.ToggleTest();
        Assert.Equal(new CoreCommand.AudioTestStop(SoundModel.StopId), sent.Commands[^1]);
        sound.Apply(Ev.Of("""{"type":"audio.tested","ref":"sound.test","ended":"stopped","heard":true,"peak":0.8}"""));
        Assert.False(sound.IsTesting);
        Assert.Equal(0, sound.TestLevel); // empty and still once it ends
        Assert.Equal("No test running", sound.LevelSpoken);
        Assert.Equal("Inkwell heard you.", sound.TestLine);
        Assert.False(sound.TestLineIsProblem);
        sound.ToggleTest();
        sound.Apply(Ev.Of("""{"type":"audio.tested","ref":"sound.test","ended":"done","heard":false,"peak":0.01}"""));
        Assert.StartsWith("Not hearing you?", sound.TestLine, StringComparison.Ordinal);
        Assert.True(sound.TestLineIsProblem);
        sound.ToggleTest();
        sound.Apply(Ev.Of("""{"type":"audio.tested","ref":"sound.test","ended":"meeting","heard":true,"peak":0.5}"""));
        Assert.Equal("Stopped: a meeting started recording.", sound.TestLine);
    }

    /// <summary>A result goes with another mic, or other devices, unless it says why the test stopped.</summary>
    [Fact]
    public void ATestsResultGoesWithTheMicItWasAboutButKeepsWhyItFailed()
    {
        var sound = new SoundModel(new Sent().Send);
        sound.Apply(Devices());
        sound.ToggleTest();
        sound.Apply(Ev.Of("""{"type":"audio.tested","ref":"sound.test","ended":"done","heard":true,"peak":0.7}"""));
        sound.Apply(Devices(type: "audio.devices_changed"));
        Assert.Equal(SoundModel.TestState.Idle, sound.Test);
        sound.ToggleTest();
        sound.Apply(Ev.Of("""{"type":"audio.tested","ref":"sound.test","ended":"failed","heard":true,"peak":0.4,"message":"the microphone M went away"}"""));
        sound.Apply(Devices(type: "audio.devices_changed"));
        Assert.Equal("The test stopped: the microphone M went away.", sound.TestLine);
        sound.ChooseInput("hs");
        Assert.Equal(SoundModel.TestState.Idle, sound.Test);
    }

    [Fact]
    public void ATestRefusedWhileAMeetingRecordsSaysItWaitsAndNoTestIsEverStuck()
    {
        var sent = new Sent();
        var sound = new SoundModel(sent.Send);
        sound.Load();
        sound.ToggleTest();
        sound.Apply(Failed("audio.test", SoundModel.TestId, "meeting_recording"));
        Assert.False(sound.IsTesting);
        Assert.Equal("The test waits until the meeting ends.", sound.TestLine);
        Assert.True(sound.TestLineIsProblem);
        sound.ToggleTest();
        Assert.NotEqual("The test waits until the meeting ends.", sound.TestLine); // a new try clears it
        sound.Apply(Ev.Of("""{"type":"core.stopped"}"""));
        Assert.Equal(SoundModel.TestState.Idle, sound.Test);
        // A core started again while Sound shows is asked for the devices.
        sound.Apply(Ev.Of("""{"type":"core.ready","abi":1,"version":"1.0.0"}"""));
        Assert.Equal(new CoreCommand.AudioDevices(SoundModel.DevicesId), sent.Commands[^1]);
    }

    /// <summary>Settings went while the test was opening: its Stop found nothing, so the start is stopped when it arrives.</summary>
    [Fact]
    public void ATestOpeningAsSettingsGoesIsStopped()
    {
        var sent = new Sent();
        var sound = new SoundModel(sent.Send);
        sound.Load();
        sound.ToggleTest();
        sound.Disappeared();
        Assert.Equal(new CoreCommand.AudioTestStop(SoundModel.StopId), sent.Commands[^1]);
        sound.Apply(Failed("audio.test_stop", SoundModel.StopId));
        sent.Commands.Clear();
        sound.Apply(Ev.Of("""{"type":"audio.test_started","ref":"sound.test","mic_name":"M","mic_transport":"usb","mic_reason":"chosen","seconds":15}"""));
        Assert.Equal([new CoreCommand.AudioTestStop(SoundModel.StopId)], sent.Commands);
    }

    [Fact]
    public void TheCommandsAreTheJsonTheCoreReadsAndTheScreensRouteSound()
    {
        Assert.Equal("""{"cmd":"audio.devices","id":"a"}""", new CoreCommand.AudioDevices("a").Json);
        Assert.Equal("""{"cmd":"audio.test","id":"t"}""", new CoreCommand.AudioTest("t").Json);
        Assert.Equal("""{"cmd":"audio.test_stop","id":"s"}""", new CoreCommand.AudioTestStop("s").Json);
        Assert.Contains("\"key\":\"audio.output\"", new CoreCommand.SettingSet(ShellSetting.AudioOutput, "default").Json, StringComparison.Ordinal);
        Assert.True(SoundModel.Handles(Failed("audio.test", SoundModel.TestId)));
        Assert.True(SoundModel.Handles(Failed("setting.set", "setting:audio.input")));
        Assert.False(SoundModel.Handles(Failed("audio.test", "other")));
        var screens = new ScreenModels(new Sent().Send);
        screens.Apply([Devices()]);
        Assert.Equal("auto", screens.Sound.Current!.Input);
        Assert.True(screens.Handles(Failed("audio.devices", SoundModel.DevicesId)));
    }
}
