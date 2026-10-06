// Settings > Sound, as the Mac's SoundModel: the microphone Inkwell records with (one for
// dictation, meetings and the mic test alike: audio.input), what Automatic picks now and why, the
// output Record now records (audio.output: Windows only, since its far end is device loopback),
// and the mic test with its level.
//
// The core holds the choices and resolves them against the devices connected now; this model only
// shows what it says. The lists follow the devices as they come and go (audio.devices_changed,
// pushed by the core), so nothing here polls. The test's level arrives about ten times a second
// while a test runs, and not at all otherwise: the meter moves only then (architecture rule 9).
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class SoundModel(Action<CoreCommand> send) : ObservableModel
{
    /// <summary>What the core last said about the devices.</summary>
    /// <param name="Input">The mic choice: "auto", or a device's id.</param>
    /// <param name="WantedName">The chosen mic's name as remembered, shown even while it is not connected.</param>
    /// <param name="Automatic">What Automatic records now; null when there is no microphone.</param>
    /// <param name="Using">The mic a take, a meeting or a test would open now, and why.</param>
    /// <param name="Outputs">The outputs, default first; null where there is no output picker.</param>
    /// <param name="Output">The output choice: "default", or a device's id.</param>
    /// <param name="OutputUsing">The output Record now records, and why.</param>
    public sealed record Devices(
        IReadOnlyList<AudioDevice> Inputs, string Input, string? WantedName, AudioInput? Automatic, AudioInput? Using,
        IReadOnlyList<AudioDevice>? Outputs, string Output, string? OutputWantedName, AudioOutput? OutputUsing);

    /// <summary>A line of a picker: "auto", "default" or a device's id, and what it says.</summary>
    public sealed record Choice(string Id, string Title);

    /// <summary>The mic test.</summary>
    public enum TestState
    {
        Idle,
        /// <summary>Asked for; the mic is opening.</summary>
        Starting,
        Running,
        Ended,
    }

    /// <summary>The command ids; every answer and failure carries one back.</summary>
    public const string DevicesId = "sound.devices";
    public const string TestId = "sound.test";
    public const string StopId = "sound.test_stop";
    public static readonly string InputSettingId = ShellSetting.AudioInput.CommandId();
    public static readonly string OutputSettingId = ShellSetting.AudioOutput.CommandId();

    /// <summary>The devices could not be listed: gone with the next list.</summary>
    private string? listProblem;
    /// <summary>A mic choice the core refused: said until the user picks again (the list read after it must not hide why the picker snapped back).</summary>
    private string? inputProblem;
    /// <summary>The same for the output.</summary>
    private string? outputProblem;
    /// <summary>Settings shows Sound (between Load and Disappeared).</summary>
    private bool visible;

    public Devices? Current { get; private set; }

    public TestState Test { get; private set; }

    /// <summary>The mic a running test records with.</summary>
    public string? TestMic { get; private set; }

    /// <summary>The level over the last tenth of a second, 0 to 1, while a test runs.</summary>
    public double TestLevel { get; private set; }

    /// <summary>How the last test ended, whether it heard more than a quiet room, and the core's words when it failed.</summary>
    public (AudioTestEnd Ended, bool Heard, string? Message)? TestResult { get; private set; }

    /// <summary>Why the test could not start (a meeting records).</summary>
    public string? TestRefused { get; private set; }

    /// <summary>What is said under the microphone picker.</summary>
    public string? InputProblem => inputProblem ?? listProblem;

    /// <summary>What is said under the output picker.</summary>
    public string? OutputProblem => outputProblem;

    public bool IsTesting => Test is TestState.Starting or TestState.Running;

    /// <summary>Whether there is an output picker (Windows' core lists outputs).</summary>
    public bool HasOutputs => Current?.Outputs is not null;

    /// <summary>Settings shows Sound: what the devices are now.</summary>
    public void Load()
    {
        visible = true;
        send(new CoreCommand.AudioDevices(DevicesId));
    }

    /// <summary>The user picked mic <paramref name="id"/> ("auto" or a device's). Shown at once; the core checks the device is still connected.</summary>
    public void ChooseInput(string id)
    {
        if (Current is not { } devices || id == devices.Input)
        {
            return;
        }
        inputProblem = null;
        // What records is unknown until the core answers (a stand-in's line would name the new mic
        // as missing meanwhile), and a test's result was about the mic before.
        Current = devices with { Input = id, WantedName = devices.Inputs.FirstOrDefault(d => d.Id == id)?.Name, Using = null };
        ForgetTestResult();
        Changed();
        send(new CoreCommand.SettingSet(ShellSetting.AudioInput, id));
    }

    /// <summary>The user picked output <paramref name="id"/> ("default" or a device's).</summary>
    public void ChooseOutput(string id)
    {
        if (Current is not { } devices || id == devices.Output)
        {
            return;
        }
        outputProblem = null;
        Current = devices with { Output = id, OutputWantedName = devices.Outputs?.FirstOrDefault(d => d.Id == id)?.Name, OutputUsing = null };
        Changed();
        send(new CoreCommand.SettingSet(ShellSetting.AudioOutput, id));
    }

    /// <summary>Test, or Stop while one runs.</summary>
    public void ToggleTest()
    {
        if (IsTesting)
        {
            send(new CoreCommand.AudioTestStop(StopId));
            return;
        }
        TestRefused = null;
        TestResult = null;
        TestLevel = 0;
        Test = TestState.Starting;
        Changed();
        send(new CoreCommand.AudioTest(TestId));
    }

    /// <summary>
    /// Settings goes away: a running test stops (it would keep the mic open until its time is up),
    /// and a refused choice is old news when it comes back.
    /// </summary>
    public void Disappeared()
    {
        visible = false;
        if (inputProblem is not null || outputProblem is not null)
        {
            inputProblem = outputProblem = null;
            Changed();
        }
        if (IsTesting)
        {
            send(new CoreCommand.AudioTestStop(StopId));
        }
    }

    /// <summary>Whether this model shows the failure: its commands and its two settings (matched by id).</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Id is DevicesId or TestId or StopId
            || (failed.Id is string id && (id == InputSettingId || id == OutputSettingId));
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case CoreReady or CoreStopped:
                // A test the core never ended (it stopped, or started again) is over.
                if (IsTesting || TestRefused is not null)
                {
                    Test = TestState.Idle;
                    TestRefused = null;
                    Changed();
                }
                // A core started again while Settings shows: the devices as it sees them.
                if (e is CoreReady && visible)
                {
                    Load();
                }
                break;
            case AudioDevices d:
                Current = new(d.Inputs, d.Input, d.Wanted?.Name, d.Automatic, d.Using, d.Outputs, d.Output ?? "default", d.OutputWanted?.Name, d.OutputUsing);
                listProblem = null;
                Changed();
                break;
            case AudioDevicesChanged d:
                Current = new(d.Inputs, d.Input, d.Wanted?.Name, d.Automatic, d.Using, d.Outputs, d.Output ?? "default", d.OutputWanted?.Name, d.OutputUsing);
                ForgetTestResult(keepingWhy: true);
                Changed();
                break;
            // Only this screen's test (its ref): a late answer to one stopped earlier never ends the next.
            case AudioTestStarted started when started.Ref == TestId && IsTesting:
                Test = TestState.Running;
                TestMic = started.MicName;
                TestLevel = 0;
                Changed();
                // Settings went while it was opening: its Stop reached the core first and found nothing.
                if (!visible)
                {
                    send(new CoreCommand.AudioTestStop(StopId));
                }
                break;
            case AudioTestLevel level when level.Ref == TestId && Test == TestState.Running:
                TestLevel = Math.Clamp(level.Level, 0, 1);
                Changed();
                break;
            case AudioTested tested when tested.Ref == TestId && IsTesting:
                Test = TestState.Ended;
                TestLevel = 0;
                TestResult = (tested.Ended, tested.Heard, tested.Message);
                Changed();
                break;
            case CommandFailed failed when failed.Id == TestId:
                Test = TestState.Idle;
                TestRefused = failed.Code == FailureCode.MeetingRecording
                    ? "The test waits until the meeting ends."
                    : $"Couldn't start the test: {failed.Message}";
                Changed();
                break;
            case CommandFailed failed when failed.Id == StopId:
                // No test was running: the screen already thinks so, or will when audio.tested
                // lands. (A Stop pressed while the test was still opening can reach the core
                // first; the test then runs to its end, and the button still offers Stop.)
                break;
            case CommandFailed failed when failed.Id == DevicesId:
                listProblem = $"Couldn't list the devices: {failed.Message}";
                Changed();
                break;
            case CommandFailed failed when failed.Id == InputSettingId:
                inputProblem = $"Couldn't choose that microphone: {failed.Message}";
                Changed();
                // The choice as it really is.
                Load();
                break;
            case CommandFailed failed when failed.Id == OutputSettingId:
                outputProblem = $"Couldn't choose that output: {failed.Message}";
                Changed();
                Load();
                break;
        }
    }

    /// <summary>
    /// A finished test's line is about the mic it ran on; another mic makes it stale, and so do other
    /// devices, unless it says why the test failed or stopped for a meeting (the devices change that
    /// follows a mic going mid-test must not hide why it stopped).
    /// </summary>
    private void ForgetTestResult(bool keepingWhy = false)
    {
        if (keepingWhy && TestResult is { Ended: AudioTestEnd.Failed or AudioTestEnd.Meeting })
        {
            return;
        }
        if (Test == TestState.Ended)
        {
            Test = TestState.Idle;
            TestResult = null;
        }
    }

    // What the section says.

    /// <summary>
    /// The microphone picker's lines: Automatic first (with what it records now), then each mic with
    /// how it connects, and a chosen one that is not connected, so the choice still shows.
    /// </summary>
    public IReadOnlyList<Choice> InputChoices
    {
        get
        {
            if (Current is not { } d)
            {
                return [];
            }
            var lines = new List<Choice> { new("auto", d.Automatic is { } a ? $"Automatic ({a.Name})" : "Automatic") };
            lines.AddRange(d.Inputs.Select(i => new Choice(i.Id, Title(i.Name, i.Transport))));
            if (d.Input != "auto" && d.Inputs.All(i => i.Id != d.Input))
            {
                lines.Add(new(d.Input, $"{d.WantedName ?? "The microphone you chose"} · not connected"));
            }
            return lines;
        }
    }

    /// <summary>"Their sound from": the default output first (with its name), then each output, and a chosen one that is not connected.</summary>
    public IReadOnlyList<Choice> OutputChoices
    {
        get
        {
            if (Current is not { Outputs: { } outputs } d)
            {
                return [];
            }
            var current = outputs.FirstOrDefault(o => o.IsDefault) ?? outputs.FirstOrDefault();
            var lines = new List<Choice> { new("default", current is null ? "Default output" : $"Default output ({current.Name})") };
            lines.AddRange(outputs.Select(o => new Choice(o.Id, Title(o.Name, o.Transport))));
            if (d.Output != "default" && outputs.All(o => o.Id != d.Output))
            {
                lines.Add(new(d.Output, $"{d.OutputWantedName ?? "The output you chose"} · not connected"));
            }
            return lines;
        }
    }

    /// <summary>"Headset (AirPods Pro) · Bluetooth": a device's name and how it connects, when that says something.</summary>
    public static string Title(string name, MicTransport transport) =>
        TransportWord(transport) is { } word ? $"{name} · {word}" : name;

    public static string? TransportWord(MicTransport transport) => transport switch
    {
        MicTransport.BuiltIn => "Built-in",
        MicTransport.Bluetooth => "Bluetooth",
        MicTransport.Usb => "USB",
        MicTransport.Virtual => "Virtual",
        _ => null,
    };

    /// <summary>The chosen mic is not connected, and Automatic stands in: the line that says so.</summary>
    public string? MissingLine =>
        Current is { Using: { Reason: MicReason.ChosenMissing } using } d
            ? $"{d.WantedName ?? "The microphone you chose"} isn't connected. Inkwell is using {using.Name} until it is."
            : null;

    /// <summary>The caption under the microphone picker.</summary>
    public string InputCaption
    {
        get
        {
            if (Current is not { } d)
            {
                return "Reading the microphones…";
            }
            if (MissingLine is { } missing)
            {
                return missing;
            }
            if (d.Inputs.Count == 0)
            {
                return "No microphone is connected.";
            }
            if (d.Input != "auto")
            {
                return d.Using?.Transport == MicTransport.Bluetooth
                    ? "While Inkwell listens, a classic Bluetooth headset switches to call quality, for what you hear as well as your voice."
                    : "Dictation and meetings both use it. A meeting keeps its microphone unless it goes.";
            }
            return "Automatic follows Windows' input, but with classic Bluetooth headphones it uses another microphone: theirs carries only call-quality sound.";
        }
    }

    /// <summary>The caption under the output picker.</summary>
    public string OutputCaption
    {
        get
        {
            if (Current?.OutputUsing is not { } using)
            {
                return Current?.Outputs is { Count: 0 } ? "No output is connected." : "Record now, and a call Inkwell can't hear alone, record what this output plays.";
            }
            return using.Reason switch
            {
                OutputReason.ChosenMissing =>
                    $"{Current.OutputWantedName ?? "The output you chose"} isn't connected. Inkwell records the default output until it is.",
                OutputReason.Chosen =>
                    $"Record now, and a call Inkwell can't hear alone, record what {using.Name} plays, even when Windows' default changes.",
                _ => "Record now, and a call Inkwell can't hear alone, record what the default output plays, and follow it when it changes.",
            };
        }
    }

    /// <summary>The line under the test button.</summary>
    public string TestLine
    {
        get
        {
            if (TestRefused is { } refused)
            {
                return refused;
            }
            return Test switch
            {
                TestState.Idle => "Speak for a few seconds to see the level. Nothing is kept.",
                TestState.Starting => "Opening the microphone…",
                TestState.Running => $"Listening with {TestMic}…",
                _ => TestResult switch
                {
                    { Ended: AudioTestEnd.Meeting } => "Stopped: a meeting started recording.",
                    { Ended: AudioTestEnd.Failed, Message: var message } => $"The test stopped: {message ?? "the microphone failed"}.",
                    { Heard: true } => "Inkwell heard you.",
                    _ => "Not hearing you? Check that this is the microphone you speak into, and that it isn't muted.",
                },
            };
        }
    }

    /// <summary>Whether the test's line is a problem.</summary>
    public bool TestLineIsProblem =>
        TestRefused is not null
        || (Test == TestState.Ended && TestResult is { Ended: AudioTestEnd.Failed } or { Ended: AudioTestEnd.Done or AudioTestEnd.Stopped, Heard: false });

    /// <summary>The meter's value for Narrator.</summary>
    public string LevelSpoken => Test == TestState.Running ? $"{(int)Math.Round(TestLevel * 100)} percent" : "No test running";
}
