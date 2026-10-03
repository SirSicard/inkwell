// "Record a shortcut…" on Windows (the Mac's ShortcutRecorderModel): how a key reads, what a press
// captures, and the recording's steps through the core's hotkey.check, with dictation paused
// meanwhile.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class ShortcutRecorderTests
{
    private sealed class FakeWakes : IWakeScheduler
    {
        public List<(TimeSpan Delay, Action Wake, Handle Handle)> Scheduled { get; } = [];

        public IDisposable After(TimeSpan delay, Action wake)
        {
            var handle = new Handle();
            Scheduled.Add((delay, wake, handle));
            return handle;
        }

        public void RunDue()
        {
            foreach (var (_, wake, handle) in Scheduled.ToList())
            {
                if (!handle.Disposed)
                {
                    wake();
                }
            }
        }

        public sealed class Handle : IDisposable
        {
            public bool Disposed { get; private set; }

            public void Dispose() => Disposed = true;
        }
    }

    private sealed class Rig
    {
        public Sent Sent { get; } = new();
        public FakeWakes Wakes { get; } = new();
        public List<string> Announced { get; } = [];
        public List<string> EditKeys { get; } = [];
        public DictationModel Dictation { get; }
        public ShortcutRecorderModel Recorder { get; }

        public Rig(string key = "right_control", string? editKey = null)
        {
            Dictation = new DictationModel(Sent.Send);
            Dictation.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.enabled","value":"on"}"""));
            Dictation.Apply(Ev.Of(editKey is null
                ? $$"""{"type":"dictation.ready","key":"{{key}}"}"""
                : $$"""{"type":"dictation.ready","key":"{{key}}","edit_key":"{{editKey}}"}"""));
            Recorder = new ShortcutRecorderModel(Sent.Send, Dictation, EditKeys.Add, Wakes) { Announce = Announced.Add };
            Sent.Commands.Clear();
        }

        public void Press(params ShortcutCapture.Input[] inputs)
        {
            foreach (var input in inputs)
            {
                Recorder.Feed(input);
            }
        }

        public CoreCommand.HotkeyCheck Check => Sent.Commands.OfType<CoreCommand.HotkeyCheck>().Last();

        public void Answer(string canonicalOrNull, string? reason = null) => Recorder.Apply(Ev.Of(canonicalOrNull.Length > 0
            ? $$"""{"type":"hotkey.checked","binding":"{{Check.Binding}}","ok":true,"canonical":"{{canonicalOrNull}}","ref":"{{Check.Ref}}"}"""
            : $$"""{"type":"hotkey.checked","binding":"{{Check.Binding}}","ok":false,"reason":"{{reason}}","ref":"{{Check.Ref}}"}"""));
    }

    private static ShortcutCapture.Input.KeyDown Down(string modifier, bool repeat = false) => new ShortcutCapture.Input.KeyDown(CapturedKey.Side(modifier), repeat);

    private static ShortcutCapture.Input.KeyUp Up(string modifier) => new ShortcutCapture.Input.KeyUp(CapturedKey.Side(modifier));

    private static ShortcutCapture.Input.KeyDown Down(uint vk, bool repeat = false) => new ShortcutCapture.Input.KeyDown(CapturedKey.Of(vk), repeat);

    private static ShortcutCapture.Input.KeyUp Up(uint vk) => new ShortcutCapture.Input.KeyUp(CapturedKey.Of(vk));

    private const uint Space = 0x20;
    private const uint A = 0x41;
    private const uint F13 = 0x7C;

    [Fact]
    public void KeysReadInWindowsNotationWithTheLayoutsLabels()
    {
        Assert.Equal(new DictationKey("ctrl+shift+space", "Ctrl+Shift+Space", "Ctrl+Shift+Space"), KeyNotation.Describe("ctrl+shift+space"));
        Assert.Equal("Right Ctrl", KeyNotation.Describe("right_control").Cap);
        Assert.Equal("Right Alt", KeyNotation.Describe("right_alt").Cap);
        Assert.Equal("Right Windows key", KeyNotation.Describe("right_win").Name);
        Assert.Equal("F13", KeyNotation.Describe("f13").Cap);
        Assert.Equal("Ctrl+Alt+Delete", KeyNotation.Describe("ctrl+alt+forward_delete").Cap);
        Assert.Equal("Ctrl+Backspace", KeyNotation.Describe("ctrl+delete").Cap);
        Assert.Equal("Ctrl+Shift", KeyNotation.Describe("ctrl+shift").Cap);
        Assert.Equal("Left Alt", KeyNotation.Describe("left_alt").Cap);
        Assert.Equal("nonsense", KeyNotation.Describe("nonsense").Cap);
        // The layout labels a typing key: a layout with Ö on the semicolon key shows Ö.
        Assert.Equal("Ctrl+Ö", KeyNotation.Describe("ctrl+semicolon", vk => vk == 0xBA ? "ö" : null).Cap);
        Assert.Equal("Ctrl+Space", KeyNotation.Describe("ctrl+space", _ => " ").Cap); // only typed keys take a label
        Assert.Equal("Alt+A", KeyNotation.Describe("alt+a", _ => "a").Cap);
        // Every key has one virtual key, and back.
        Assert.Equal(KeyNotation.AllKeys.Count, KeyNotation.AllKeys.Select(k => k.Vk).Distinct().Count());
        Assert.All(KeyNotation.AllKeys, k => Assert.Equal(k, KeyNotation.ForVk(k.Vk)));
        Assert.Null(KeyNotation.ForVk(0x60)); // the keypad
    }

    [Fact]
    public void KnownShortcutsAreSaidToClashWithoutPromisingAll()
    {
        Assert.Equal("Win+L is the shortcut for locking the PC, so the two may clash.", KeyNotation.Clash("win+l"));
        Assert.Equal("Alt+Tab is the shortcut for switching windows, so the two may clash.", KeyNotation.Clash("alt+tab"));
        Assert.Equal("Ctrl+Alt+Delete is the shortcut for the Ctrl+Alt+Delete screen, so the two may clash.", KeyNotation.Clash("ctrl+alt+forward_delete"));
        Assert.Equal("Win+Space is the shortcut for switching the keyboard layout, so the two may clash.", KeyNotation.Clash("win+space"));
        Assert.Equal("Shift+F10 is the shortcut for the context menu, so the two may clash.", KeyNotation.Clash("shift+f10"));
        Assert.Equal("Ctrl+Z is Undo in most apps; while dictation is on, Inkwell takes it from them.", KeyNotation.Clash("ctrl+z"));
        Assert.Null(KeyNotation.Clash("ctrl+shift+space"));
        Assert.Null(KeyNotation.Clash("right_control"));
        // By what is shown: a layout that puts Z where Y is still clashes with Undo.
        Assert.Equal("Ctrl+Z is Undo in most apps; while dictation is on, Inkwell takes it from them.", KeyNotation.Clash("ctrl+y", "Ctrl+Z"));
    }

    [Fact]
    public void APressIsCapturedAsTheCoreNamesIt()
    {
        static string? Capture(params ShortcutCapture.Input[] inputs)
        {
            var capture = new ShortcutCapture();
            ShortcutCapture.Outcome outcome = new ShortcutCapture.Outcome.Listening();
            // The recorder stops listening at the first outcome, as here.
            foreach (var input in inputs)
            {
                outcome = capture.Feed(input);
                if (outcome is not ShortcutCapture.Outcome.Listening)
                {
                    break;
                }
            }
            return outcome switch
            {
                ShortcutCapture.Outcome.Captured c => c.Token,
                ShortcutCapture.Outcome.Cancelled => "<cancelled>",
                ShortcutCapture.Outcome.UnknownKey => "<unknown>",
                _ => null,
            };
        }

        Assert.Equal("right_control", Capture(Down("right_control"), Up("right_control")));
        Assert.Equal("right_alt", Capture(Down("right_alt"), Up("right_alt")));
        Assert.Equal("left_alt", Capture(Down("left_alt"), Up("left_alt")));
        // AltGr: left Ctrl and right Alt together, as Windows sends them.
        Assert.Equal("right_alt", Capture(Down("left_control"), Down("right_alt"), Up("left_control"), Up("right_alt")));
        // ...and as WinUI reports it there (seen on a Swedish layout): right Alt's press marked a
        // repeat, and no right Alt up.
        Assert.Equal("right_alt", Capture(Down("left_control"), Down("right_alt", repeat: true), Up("left_control")));
        Assert.Null(Capture(Down("right_alt"), Down("right_alt", repeat: true))); // a held modifier's repeat
        Assert.Equal("ctrl+shift+space", Capture(Down("left_control"), Down("right_shift"), Down(Space)));
        Assert.Equal("f13", Capture(Down(F13)));
        Assert.Equal("a", Capture(Down(A)));
        Assert.Equal("alt+win+a", Capture(Down("left_win"), Down("left_alt"), Down(A)));
        Assert.Equal("ctrl+shift", Capture(Down("left_control"), Down("left_shift"), Up("left_shift"), Up("left_control")));
        Assert.Equal("caps_lock", Capture(Down(CapturedKey.CapsLock)));
        Assert.Equal("<cancelled>", Capture(Down(CapturedKey.Escape)));
        Assert.Equal("ctrl+escape", Capture(Down("left_control"), Down(CapturedKey.Escape)));
        Assert.Equal("<unknown>", Capture(Down(0x60)));
        Assert.Null(Capture(Down("right_control"), Down(0xA3, repeat: true))); // still listening
        Assert.Null(Capture(Down("right_control"), Down("right_shift"), Up("right_shift"))); // right Ctrl still down
    }

    /// <summary>
    /// Recording a key: dictation pauses (its key can't start a take), the press is checked by the
    /// core and saved only in the spelling it answers, and dictation comes back on the new key.
    /// </summary>
    [Fact]
    public void ARecordedKeyIsCheckedThenSavedAndDictationComesBack()
    {
        var rig = new Rig();
        rig.Recorder.Toggle(ShortcutTarget.Dictation);
        Assert.Equal(ShortcutTarget.Dictation, rig.Recorder.Recording);
        Assert.Equal("Press the keys… (Esc cancels)", rig.Recorder.ButtonTitle(ShortcutTarget.Dictation));
        Assert.Equal("Record a shortcut…", rig.Recorder.ButtonTitle(ShortcutTarget.Edit));
        // Narrator hears what each button is for now (the Mac's labels).
        Assert.Equal("Recording a shortcut for dictation", rig.Recorder.ButtonName(ShortcutTarget.Dictation));
        Assert.StartsWith("Press the keys you want", rig.Recorder.ButtonHint(ShortcutTarget.Dictation), StringComparison.Ordinal);
        Assert.Equal("Record a shortcut for editing a selection", rig.Recorder.ButtonName(ShortcutTarget.Edit));
        Assert.IsType<CoreCommand.DictationDisable>(Assert.Single(rig.Sent.Commands));
        Assert.Equal("Dictation is paused while you record a shortcut.", rig.Dictation.Status);
        Assert.Equal("Recording a shortcut for the dictation key. Press the keys. Escape on its own cancels.", rig.Announced[^1]);
        // Nothing turns dictation on while recording.
        rig.Dictation.Enable();
        rig.Dictation.AppBecameActive();
        Assert.Single(rig.Sent.Commands);

        rig.Press(Down("left_control"), Down("left_shift"), Down(Space));
        Assert.Equal("ctrl+shift+space", rig.Check.Binding);
        Assert.StartsWith(ShortcutRecorderModel.RefPrefix, rig.Check.Ref, StringComparison.Ordinal);
        Assert.Equal("Cancel", rig.Recorder.ButtonTitle(ShortcutTarget.Dictation));
        Assert.Equal("Cancel checking the shortcut for dictation", rig.Recorder.ButtonName(ShortcutTarget.Dictation));
        Assert.Equal(new ShortcutMessage("Checking Ctrl+Shift+Space…", false), rig.Recorder.Message(ShortcutTarget.Dictation));
        Assert.Equal(TimeSpan.FromSeconds(5), rig.Wakes.Scheduled.Single().Delay);

        rig.Answer("ctrl+shift+space");
        var set = rig.Sent.Commands.OfType<CoreCommand.SettingSet>().Single();
        Assert.Equal(ShellSetting.DictationKey, set.Key);
        Assert.Equal("ctrl+shift+space", set.Value);
        Assert.IsType<CoreCommand.DictationEnable>(rig.Sent.Commands[^1]); // after the save
        Assert.Null(rig.Recorder.Message(ShortcutTarget.Dictation));
        Assert.Equal("Dictation key set to Ctrl+Shift+Space.", rig.Announced[^1]);
        Assert.False(rig.Dictation.SuspendedForRecording);
        rig.Wakes.RunDue(); // the timeout was cancelled: nothing more
        Assert.Null(rig.Recorder.Message(ShortcutTarget.Dictation));
    }

    [Fact]
    public void ARefusedKeySaysWhyAndTheOldKeyStays()
    {
        var rig = new Rig();
        rig.Recorder.Start(ShortcutTarget.Dictation);
        rig.Press(Down(A));
        rig.Answer("", "that key on its own would stop working everywhere else; add Ctrl, Alt or Win");
        Assert.Equal(
            new ShortcutMessage("Can't use A: that key on its own would stop working everywhere else; add Ctrl, Alt or Win.", true),
            rig.Recorder.Message(ShortcutTarget.Dictation));
        Assert.Empty(rig.Sent.Commands.OfType<CoreCommand.SettingSet>());
        Assert.IsType<CoreCommand.DictationEnable>(rig.Sent.Commands[^1]);
        Assert.Equal(rig.Recorder.Message(ShortcutTarget.Dictation)!.Text, rig.Announced[^1]);
    }

    [Fact]
    public void ACheckWithNoAnswerGivesUpAndAFailedOneSaysSo()
    {
        var rig = new Rig();
        rig.Recorder.Start(ShortcutTarget.Dictation);
        rig.Press(Down("right_alt"), Up("right_alt"));
        var late = rig.Check;
        rig.Wakes.RunDue();
        Assert.Equal(
            new ShortcutMessage("Couldn't check that shortcut in time. The key before still works.", true),
            rig.Recorder.Message(ShortcutTarget.Dictation));
        Assert.Null(rig.Recorder.Checking);
        // The late answer changes nothing.
        rig.Recorder.Apply(Ev.Of($$"""{"type":"hotkey.checked","binding":"right_alt","ok":true,"canonical":"right_alt","ref":"{{late.Ref}}"}"""));
        Assert.Empty(rig.Sent.Commands.OfType<CoreCommand.SettingSet>());

        rig.Recorder.Start(ShortcutTarget.Dictation);
        rig.Press(Down("right_alt"), Up("right_alt"));
        var failed = Ev.Of($$"""{"type":"command.failed","command":"hotkey.check","id":"{{rig.Check.Ref}}","message":"the core stopped"}""");
        Assert.True(ShortcutRecorderModel.Handles((CommandFailed)failed));
        rig.Recorder.Apply(failed);
        Assert.Equal(
            new ShortcutMessage("Couldn't check that shortcut. The key before still works.", true),
            rig.Recorder.Message(ShortcutTarget.Dictation));
    }

    [Fact]
    public void EscapeOrCancelWhileCheckingChangesNothing()
    {
        var rig = new Rig();
        rig.Recorder.Start(ShortcutTarget.Dictation);
        rig.Press(Down(CapturedKey.Escape));
        Assert.Null(rig.Recorder.Recording);
        Assert.Equal("Recording cancelled. The key is unchanged.", rig.Announced[^1]);
        Assert.IsType<CoreCommand.DictationEnable>(rig.Sent.Commands[^1]);

        rig.Recorder.Toggle(ShortcutTarget.Dictation);
        rig.Press(Down("right_alt"), Up("right_alt"));
        var check = rig.Check;
        rig.Recorder.Toggle(ShortcutTarget.Dictation); // Cancel while checking
        Assert.Null(rig.Recorder.Checking);
        Assert.Null(rig.Recorder.Message(ShortcutTarget.Dictation));
        rig.Recorder.Apply(Ev.Of($$"""{"type":"hotkey.checked","binding":"right_alt","ok":true,"canonical":"right_alt","ref":"{{check.Ref}}"}"""));
        Assert.Empty(rig.Sent.Commands.OfType<CoreCommand.SettingSet>());
        // Feeding after the recording ended is not the recorder's.
        Assert.False(rig.Recorder.Feed(Down(A)));
    }

    [Fact]
    public void TheTwoKeysAreNeverOne()
    {
        var rig = new Rig(key: "right_control", editKey: "right_alt");
        rig.Recorder.Start(ShortcutTarget.Dictation);
        rig.Press(Down("right_alt"), Up("right_alt"));
        rig.Answer("right_alt");
        Assert.Equal(
            new ShortcutMessage("Right Alt is the edit key. Pick another, or change the edit key first.", true),
            rig.Recorder.Message(ShortcutTarget.Dictation));
        Assert.Empty(rig.Sent.Commands.OfType<CoreCommand.SettingSet>());

        rig.Recorder.Start(ShortcutTarget.Edit);
        rig.Press(Down("right_control"), Up("right_control"));
        rig.Answer("right_control");
        Assert.Equal(new ShortcutMessage("Right Ctrl is the dictation key. Pick another.", true), rig.Recorder.Message(ShortcutTarget.Edit));
        Assert.Empty(rig.EditKeys);

        rig.Recorder.Start(ShortcutTarget.Edit);
        rig.Press(Down("left_control"), Down("left_alt"), Down(A));
        rig.Answer("ctrl+alt+a");
        Assert.Equal(["ctrl+alt+a"], rig.EditKeys);
        Assert.Equal("Edit key chosen: Ctrl+Alt+A.", rig.Announced[^1]);
    }

    [Fact]
    public void ASavedKeyWithAKnownClashSaysSo()
    {
        var rig = new Rig();
        rig.Recorder.Start(ShortcutTarget.Dictation);
        rig.Press(Down("left_win"), Down(0x4C));
        rig.Answer("win+l");
        Assert.Equal(
            new ShortcutMessage("Win+L is the shortcut for locking the PC, so the two may clash.", true),
            rig.Recorder.Message(ShortcutTarget.Dictation));
        Assert.Equal("Dictation key set to Win+L. Win+L is the shortcut for locking the PC, so the two may clash.", rig.Announced[^1]);
    }

    /// <summary>A core that stops ends the recording and the pause: the restarted core turns dictation on from its switch.</summary>
    [Fact]
    public void ACoreStopClearsTheRecordingAndThePause()
    {
        var rig = new Rig();
        rig.Recorder.Start(ShortcutTarget.Dictation);
        rig.Press(Down("right_alt"), Up("right_alt"));
        var stopped = Ev.Of("""{"type":"core.stopped"}""");
        rig.Recorder.Apply(stopped);
        rig.Dictation.Apply(stopped);
        Assert.Null(rig.Recorder.Checking);
        Assert.False(rig.Dictation.SuspendedForRecording);
    }

    /// <summary>A refused key's line says what to do, the Mac's words.</summary>
    [Fact]
    public void ARefusedKeyLineOffersToRecordOne()
    {
        var dictation = new DictationModel(new Sent().Send);
        dictation.Apply(Ev.Of("""{"type":"dictation.off","reason":"key_refused","message":"the Fn key never reaches Windows"}"""));
        Assert.Equal("That key can't be used here: the Fn key never reaches Windows. Pick another, or record a shortcut.", dictation.Status);
        dictation.Apply(Ev.Of("""{"type":"dictation.off","reason":"key_refused"}"""));
        Assert.Equal("That key can't be used here. Pick another, or record a shortcut.", dictation.Status);
    }
}
