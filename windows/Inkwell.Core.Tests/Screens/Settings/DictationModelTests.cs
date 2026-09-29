// Settings > Voice: the keys drive the core, the switch is kept, and every failure reads as such
// (the Settings half of the Mac's DictationModelTests, and DictationScreensTests). The Drop's
// per-take texts (the Mac's TheDropSaysWhatBecameOfATakeThatDidNotGoIn, and the note asserts in
// the lost-key test) belong to the Drop step.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class DictationModelTests
{
    private static readonly TimeSpan TwoHours = TimeSpan.FromHours(2);

    private static bool IsEnable(CoreCommand c) => c is CoreCommand.DictationEnable;

    /// <summary>
    /// Windows: the core never answers needs_accessibility here (typing into other apps needs no
    /// permission), but the model reads it as the Mac does if it ever comes.
    /// </summary>
    [Fact]
    public void TheCoreIsAskedToHoldTheKeysOnceReadyAndAgainAfterAccessibilityIsGranted()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send, () => TwoHours);
        dictation.Enable();
        Assert.Equal([new CoreCommand.DictationEnable(120, "dictation:1")], sent.Commands);
        dictation.Apply(Ev.Of("""{"type":"dictation.off","reason":"needs_accessibility","message":"permission not granted: Accessibility","ref":"dictation:1"}"""));
        Assert.Equal(new DictationState.Off(DictationOffReason.NeedsAccessibility, "permission not granted: Accessibility"), dictation.State);
        Assert.True(dictation.IsProblem);
        Assert.Contains("Type for you", dictation.Status, StringComparison.Ordinal);
        // Back in the app: asked again.
        dictation.AppBecameActive();
        Assert.Equal(new CoreCommand.DictationEnable(120, "dictation:2"), sent.Commands[^1]);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control","ref":"dictation:2"}"""));
        Assert.Equal(new DictationState.Live("right_control", null), dictation.State);
        Assert.Equal("Hold Right Ctrl, speak, let go.", dictation.Status);
        Assert.False(dictation.IsProblem);
        // Live: coming back asks nothing.
        var before = sent.Commands.Count;
        dictation.AppBecameActive();
        Assert.Equal(before, sent.Commands.Count);
    }

    [Fact]
    public void PickingAKeySavesItAndTheCoresAnswerIsWhatShows()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send);
        dictation.Load();
        Assert.Equal(
            [new CoreCommand.SettingGet(ShellSetting.DictationEnabled), new CoreCommand.SettingGet(ShellSetting.DictationKey), new CoreCommand.SettingGet(ShellSetting.DictationEditKey)],
            sent.Commands);
        dictation.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.key"}"""));
        Assert.Equal("right_control", dictation.CurrentKey); // never set: the Windows default
        Assert.Null(dictation.EditKey);
        dictation.SetKey("right_alt");
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.DictationKey, "right_alt"), sent.Commands[^1]);
        dictation.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.key","value":"right_alt"}"""));
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_alt"}"""));
        Assert.Equal("right_alt", dictation.CurrentKey);
        Assert.Equal("Hold Right Alt, speak, let go.", dictation.Status);
        dictation.SetEditKey("right_shift");
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.DictationEditKey, "right_shift"), sent.Commands[^1]);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_alt","edit_key":"right_shift"}"""));
        Assert.Equal("right_shift", dictation.EditKey);
        dictation.SetEditKey(null);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.DictationEditKey, "off"), sent.Commands[^1]);
        // A key the core could not hold for the edit: said, and dictation goes on.
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_alt","edit_key_error":"the edit key is the dictation key; pick another"}"""));
        Assert.Equal("the edit key is the dictation key; pick another", dictation.EditKeyProblem);
        Assert.Equal("The edit key isn't held: the edit key is the dictation key; pick another", dictation.EditKeyProblemLine);
        Assert.Equal(new DictationState.Live("right_alt", null), dictation.State);
    }

    /// <summary>
    /// The edit picker offers every key but the dictation key, so its selection must never be
    /// that key, even when the stored settings collide before the core has answered.
    /// </summary>
    [Fact]
    public void TheEditKeyIsNeverTheDictationKeyEvenBeforeTheCoreAnswers()
    {
        var dictation = new DictationModel(_ => { });
        dictation.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.key","value":"right_alt"}"""));
        dictation.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.edit_key","value":"right_alt"}"""));
        Assert.IsType<DictationState.Starting>(dictation.State);
        Assert.Null(dictation.EditKey);
        Assert.DoesNotContain(dictation.EditKeys, k => k.Token == "right_alt");
        dictation.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.edit_key","value":"right_shift"}"""));
        Assert.Equal("right_shift", dictation.EditKey);
    }

    /// <summary>Stopped after repeated failures: Settings offers to turn it on again, which asks the core.</summary>
    [Fact]
    public void DictationThatStoppedCanBeTurnedOnAgain()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send);
        dictation.Apply(Ev.Of("""{"type":"dictation.off","reason":"worker_stopped","message":"dictation stopped after repeated failures; turn it on again"}"""));
        Assert.True(dictation.CanRetry);
        dictation.Enable();
        Assert.Contains(sent.Commands, IsEnable);
        dictation.Apply(Ev.Of("""{"type":"dictation.off","reason":"needs_accessibility"}"""));
        Assert.False(dictation.CanRetry); // not a retry's to fix
        dictation.Apply(Ev.Of("""{"type":"dictation.off","reason":"unsupported"}"""));
        Assert.False(dictation.CanRetry);
    }

    /// <summary>
    /// A key Windows stopped sending is never shown as working: Settings says so until dictation is
    /// enabled again (coming back to the app tries, and Windows offers the retry), and Today lists it.
    /// </summary>
    [Fact]
    public void ALostKeyIsShownAsNotWorkingUntilDictationIsEnabledAgain()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control","edit_key":"right_alt"}"""));
        Assert.Null(dictation.EditKeyProblem);
        dictation.Apply(Ev.Of("""{"type":"dictation.edit_hotkey_lost"}"""));
        Assert.Equal(DictationModel.EditKeyLostText, dictation.EditKeyProblem);
        Assert.Equal(DictationModel.EditKeyLostText, dictation.EditKeyProblemLine);
        Assert.True(dictation.IsProblem);
        Assert.True(dictation.CanRetry); // Windows: the hook comes back when dictation is enabled again
        // Back in the app: asked again, and the answer clears it.
        var before = sent.Commands.Count;
        dictation.AppBecameActive();
        Assert.Equal(before + 1, sent.Commands.Count);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control","edit_key":"right_alt"}"""));
        Assert.Null(dictation.EditKeyProblem);
        Assert.False(dictation.IsProblem);

        dictation.Apply(Ev.Of("""{"type":"dictation.hotkey_lost"}"""));
        Assert.Equal(DictationModel.KeyLostText, dictation.Status);
        Assert.DoesNotContain("Type for you", DictationModel.KeyLostText, StringComparison.Ordinal); // no permission to check on Windows
        Assert.True(dictation.IsProblem);
        Assert.True(dictation.CanRetry);
        Assert.Equal("Turn dictation on", dictation.RetryTitle);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control"}"""));
        Assert.Equal("Hold Right Ctrl, speak, let go.", dictation.Status);

        var store = new CoreStore();
        store.Apply([Ev.Of("""{"type":"dictation.edit_hotkey_lost"}""")]);
        Assert.Equal([new NoticeKind.EditKeyLost()], store.Notices.Select(n => n.Kind));
    }

    /// <summary>A dictation command the core could not even queue or run reads as such, with a way to try again: never "Starting..." forever.</summary>
    [Fact]
    public void ADictationCommandThatFailedIsShownWithARetry()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send, () => TwoHours);
        dictation.Enable();
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"dictation.enable","id":"dictation:1","message":"a bug in the core stopped this command"}""");
        dictation.Apply(failed);
        Assert.Equal("Dictation couldn't start.", dictation.Status);
        Assert.True(dictation.IsProblem);
        Assert.True(dictation.CanRetry);
        Assert.True(DictationModel.Handles(failed));
        dictation.Retry();
        Assert.Equal(new CoreCommand.DictationEnable(120, "dictation:2"), sent.Commands[^1]);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control","ref":"dictation:2"}"""));
        Assert.False(dictation.IsProblem);

        // Turning it off failed: it still reads as on, says so, and retrying turns it off again.
        dictation.SetOn(false);
        Assert.Equal(
            [new CoreCommand.SettingSet(ShellSetting.DictationEnabled, "off"), new CoreCommand.DictationDisable("dictation:3")],
            sent.Commands.TakeLast(2));
        dictation.Apply(Ev.Of("""{"type":"command.failed","command":"dictation.disable","id":"dictation:3","message":"the queries thread has stopped"}"""));
        Assert.Equal("Dictation couldn't be turned off.", dictation.Status);
        Assert.True(dictation.IsProblem);
        dictation.Retry();
        Assert.Equal(new CoreCommand.DictationDisable("dictation:4"), sent.Commands[^1]);
        dictation.Apply(Ev.Of("""{"type":"dictation.off","reason":"disabled","ref":"dictation:4"}"""));
        Assert.Equal("Dictation is off.", dictation.Status);
        Assert.False(dictation.IsProblem);
    }

    /// <summary>The user's latest action speaks first: with the key already lost, turning dictation off and failing reads as that failure, not as the older lost key.</summary>
    [Fact]
    public void AFailedTurnOffReadsAsSuchEvenWithTheKeyLost()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control"}"""));
        dictation.Apply(Ev.Of("""{"type":"dictation.hotkey_lost"}"""));
        Assert.Equal(DictationModel.KeyLostText, dictation.Status);
        dictation.SetOn(false);
        var disable = Assert.IsType<CoreCommand.DictationDisable>(sent.Commands[^1]);
        dictation.Apply(Ev.Of($$"""{"type":"command.failed","command":"dictation.disable","id":"{{disable.Ref}}","message":"the queries thread has stopped"}"""));
        Assert.Equal("Dictation couldn't be turned off.", dictation.Status);
        Assert.Equal("Try again", dictation.RetryTitle);
    }

    /// <summary>Settings > Voice turns dictation off and on; the choice is kept, and a launch with it off holds no key.</summary>
    [Fact]
    public void DictationCanBeTurnedOffAndStaysOffAtTheNextLaunch()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send);
        dictation.Load();
        Assert.Contains(new CoreCommand.SettingGet(ShellSetting.DictationEnabled), sent.Commands);
        Assert.DoesNotContain(sent.Commands, IsEnable); // not before the switch is read
        dictation.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.enabled","value":"off"}"""));
        Assert.DoesNotContain(sent.Commands, IsEnable);
        Assert.False(dictation.IsOn);
        Assert.Equal("Dictation is off.", dictation.Status);
        Assert.Equal("Off: the keys do what they did before", dictation.SwitchCaption);
        dictation.SetOn(true);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.DictationEnabled, "on"), sent.Commands[^2]);
        Assert.True(IsEnable(sent.Commands[^1]));
        Assert.True(dictation.IsOn);

        // Never set: on, as a fresh install is.
        var fresh = new Sent();
        var first = new DictationModel(fresh.Send);
        first.Load();
        first.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.enabled"}"""));
        Assert.Contains(fresh.Commands, IsEnable);
        // A switch that cannot be read: dictation starts (the default), and Settings says so.
        var unread = new Sent();
        var third = new DictationModel(unread.Send);
        third.Load();
        third.Apply(Ev.Of("""{"type":"command.failed","command":"setting.get","id":"setting:dictation.enabled","message":"x"}"""));
        Assert.Contains(unread.Commands, IsEnable);
        Assert.Equal("Couldn't read whether dictation is on, so it is on.", third.KeyFailure);
        Assert.Equal(third.KeyFailure, third.StatusLine);
        // Coming back to the app never turns on a dictation the user turned off.
        var before = sent.Commands.Count;
        dictation.SetOn(false);
        dictation.Apply(Ev.Of("""{"type":"dictation.off","reason":"disabled"}"""));
        dictation.AppBecameActive();
        Assert.Equal(before + 2, sent.Commands.Count);
    }

    /// <summary>A key setting that could not be saved or read reads "couldn't", never as the key changed.</summary>
    [Fact]
    public void AKeySettingThatCouldNotBeSavedSaysSo()
    {
        var dictation = new DictationModel(_ => { });
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:dictation.key","message":"the library could not be written"}""");
        dictation.Apply(failed);
        Assert.Equal("Couldn't save the key. The one before still works.", dictation.KeyFailure);
        Assert.True(dictation.IsProblem);
        Assert.True(DictationModel.Handles(failed));
        dictation.Apply(Ev.Of("""{"type":"command.failed","command":"setting.get","id":"setting:dictation.edit_key","message":"x"}"""));
        Assert.Equal("Couldn't read your key settings.", dictation.KeyFailure);
        dictation.SetKey("right_control");
        Assert.Null(dictation.KeyFailure); // a new try clears it
    }

    /// <summary>Windows: the keys offered are the right-hand modifiers (fn is refused on Windows), and a chord the core holds reads spelled out.</summary>
    [Fact]
    public void TheKeysOfferedAreWindowsRightHandModifiers()
    {
        Assert.Equal(["right_control", "right_alt", "right_shift", "right_win"], DictationModel.Keys.Select(k => k.Token));
        Assert.Equal(["Right Ctrl", "Right Alt", "Right Shift", "Right Windows key"], DictationModel.Keys.Select(k => k.Name));
        Assert.Null(DictationModel.Key("fn"));
        Assert.Equal("Ctrl+Shift+Space", DictationModel.Cap("ctrl+shift+space"));
        Assert.Equal("F13", DictationModel.Cap("f13"));
        var dictation = new DictationModel(_ => { });
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"ctrl+shift+space"}"""));
        Assert.Equal("Hold Ctrl+Shift+Space, speak, let go.", dictation.Status);
        Assert.Equal(DictationModel.Keys.Count, dictation.EditKeys.Count); // a chord is not in the picker, so every key is offered for the edit
    }

    /// <summary>A view bound to the model hears about each change, and an event it does not read changes nothing.</summary>
    [Fact]
    public void OnlyItsOwnEventsChangeIt()
    {
        var dictation = new DictationModel(_ => { });
        dictation.Apply(Ev.Of("""{"type":"dictation.started","take":0,"edit":false}"""));
        Assert.Equal(0, dictation.Changes);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"right_control"}"""));
        Assert.Equal(1, dictation.Changes);
    }
}

public class DictationScreensTests
{
    /// <summary>Through the Settings models wired as ScreenModels wires them (the aggregator's core.ready loads).</summary>
    [Fact]
    public void TheCoreBeingReadyAsksForTheKeysAndTheirSettings()
    {
        var sent = new Sent();
        var screens = new SettingsHarness(sent.Send);
        screens.Apply(Ev.Of("""{"type":"core.ready","abi":2,"version":"0.0.0"}"""));
        Assert.Contains(new CoreCommand.SettingGet(ShellSetting.DictationKey), sent.Commands);
        Assert.Contains(new CoreCommand.SettingGet(ShellSetting.DictationEnabled), sent.Commands);
        screens.Apply(Ev.Of("""{"type":"setting.value","key":"dictation.enabled"}"""));
        Assert.Contains(sent.Commands, c => c is CoreCommand.DictationEnable);
    }
}
