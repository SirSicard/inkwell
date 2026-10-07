using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class MeetingShortcutTests
{
    private sealed class Wake : IWakeScheduler
    {
        public Action? Run { get; private set; }
        public IDisposable After(TimeSpan delay, Action action) { Run = action; return new Handle(); }
        private sealed class Handle : IDisposable { public void Dispose() { } }
    }
    [Fact]
    public void AFailedSaveKeepsTheConfirmedShortcut()
    {
        var sent = new Sent();
        var model = new MeetingShortcutModel(sent.Send);
        model.Apply(Ev.Of("""{"type":"meetings.shortcut.state","key":"ctrl+shift+r","active":true,"suspended":false}"""));
        model.SetKey("ctrl+alt+m");
        Assert.Equal("ctrl+shift+r", model.Key);
        model.Apply(Ev.Of("""{"type":"command.failed","command":"setting.set","id":"setting:meetings.key","message":"cannot save"}"""));
        Assert.Equal("ctrl+shift+r", model.Key);
        Assert.NotNull(model.Problem);
    }

    [Fact]
    public void OffIsConfirmedAndDoesNotAdvertiseAnAccelerator()
    {
        var model = new MeetingShortcutModel(new Sent().Send);
        model.Apply(Ev.Of("""{"type":"meetings.shortcut.state","key":"off","active":false,"suspended":false}"""));
        Assert.Equal("off", model.Key);
        Assert.Equal("", model.ShortcutLabel);
    }

    [Fact]
    public void CaptureWaitsForTheMatchingSuspensionAcknowledgement()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send);
        var meeting = new MeetingShortcutModel(sent.Send);
        var recorder = new ShortcutRecorderModel(sent.Send, dictation, _ => { }, new Wake(), meeting);
        recorder.Start(ShortcutTarget.Meeting);
        Assert.Null(recorder.Recording);
        var suspend = Assert.Single(sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>());
        recorder.Apply(Ev.Of("""{"type":"meetings.shortcut.state","key":"ctrl+shift+r","active":false,"suspended":true,"ref":"old"}"""));
        Assert.Null(recorder.Recording);
        recorder.Apply(Ev.Of($$"""{"type":"meetings.shortcut.state","key":"ctrl+shift+r","active":false,"suspended":true,"ref":"{{suspend.Id}}"}"""));
        Assert.Equal(ShortcutTarget.Meeting, recorder.Recording);
        recorder.Cancel();
        Assert.False(sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last().Suspended);
    }

    [Theory]
    [InlineData(ShortcutTarget.Dictation)]
    [InlineData(ShortcutTarget.Edit)]
    public void MeetingAndVoiceKeysCannotBeTheSame(ShortcutTarget target)
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"ctrl+space","edit_key":"right_alt"}"""));
        var meeting = new MeetingShortcutModel(sent.Send);
        var recorder = new ShortcutRecorderModel(sent.Send, dictation, _ => throw new InvalidOperationException("must not save"), new Wake(), meeting);
        recorder.Start(target);
        var suspend = sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last();
        recorder.Apply(Ev.Of($$"""{"type":"meetings.shortcut.state","key":"ctrl+shift+r","active":false,"suspended":true,"ref":"{{suspend.Id}}"}"""));
        recorder.Feed(new ShortcutCapture.Input.KeyDown(CapturedKey.Of(0x52)));
        recorder.Feed(new ShortcutCapture.Input.KeyUp(CapturedKey.Of(0x52)));
        var check = sent.Commands.OfType<CoreCommand.HotkeyCheck>().Last();
        recorder.Apply(Ev.Of($$"""{"type":"hotkey.checked","binding":"r","ok":true,"canonical":"ctrl+shift+r","ref":"{{check.Ref}}"}"""));
        Assert.Contains("meeting key", recorder.Message(target)!.Text, StringComparison.Ordinal);
        Assert.Empty(sent.Commands.OfType<CoreCommand.SettingSet>());
    }

    [Fact]
    public void MeetingCaptureCannotOverwriteTheDictationKey()
    {
        var sent = new Sent();
        var dictation = new DictationModel(sent.Send);
        dictation.Apply(Ev.Of("""{"type":"dictation.ready","key":"ctrl+space"}"""));
        var meeting = new MeetingShortcutModel(sent.Send);
        var recorder = new ShortcutRecorderModel(sent.Send, dictation, _ => { }, new Wake(), meeting);
        recorder.Start(ShortcutTarget.Meeting);
        var suspend = sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last();
        recorder.Apply(Ev.Of($$"""{"type":"meetings.shortcut.state","key":"ctrl+shift+r","active":false,"suspended":true,"ref":"{{suspend.Id}}"}"""));
        recorder.Feed(new ShortcutCapture.Input.KeyDown(CapturedKey.Of(0x20)));
        recorder.Feed(new ShortcutCapture.Input.KeyUp(CapturedKey.Of(0x20)));
        var check = sent.Commands.OfType<CoreCommand.HotkeyCheck>().Last();
        recorder.Apply(Ev.Of($$"""{"type":"hotkey.checked","binding":"space","ok":true,"canonical":"ctrl+space","ref":"{{check.Ref}}"}"""));
        Assert.Contains("dictation", recorder.Message(ShortcutTarget.Meeting)!.Text, StringComparison.Ordinal);
        Assert.Empty(sent.Commands.OfType<CoreCommand.SettingSet>());
    }

    [Fact]
    public void CancellingBeforePauseAcknowledgementDoesNotReopenCapture()
    {
        var sent = new Sent();
        var recorder = new ShortcutRecorderModel(sent.Send, new DictationModel(sent.Send), _ => { }, new Wake(), new MeetingShortcutModel(sent.Send));
        recorder.Start(ShortcutTarget.Meeting);
        var suspend = sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last();
        recorder.Cancel();
        recorder.Apply(Ev.Of($$"""{"type":"meetings.shortcut.state","key":"ctrl+shift+r","active":false,"suspended":true,"ref":"{{suspend.Id}}"}"""));
        Assert.False(recorder.Busy);
        Assert.False(sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last().Suspended);
    }

    [Fact]
    public void AConfirmedSaveChangesTheDisplayedKey()
    {
        var model = new MeetingShortcutModel(new Sent().Send);
        model.SetKey("ctrl+alt+m");
        Assert.Equal(MeetingShortcutModel.DefaultKey, model.Key);
        model.Apply(Ev.Of("""{"type":"meetings.shortcut.state","key":"ctrl+alt+m","active":true,"suspended":false}"""));
        Assert.Equal("ctrl+alt+m", model.Key);
        Assert.Equal("Ctrl+Alt+M", model.ShortcutLabel);
        Assert.True(model.Active);
    }

    [Fact]
    public void SuspendCommandCarriesABooleanAndReference()
    {
        var command = new CoreCommand.MeetingsShortcutSuspend(true, "pause:1");
        using var json = System.Text.Json.JsonDocument.Parse(command.Json);
        Assert.Equal("meetings.shortcut.suspend", json.RootElement.GetProperty("cmd").GetString());
        Assert.True(json.RootElement.GetProperty("suspended").GetBoolean());
        Assert.Equal("pause:1", json.RootElement.GetProperty("id").GetString());
    }

    [Fact]
    public void MissingPauseAcknowledgementResumesHooksAndKeepsTheKey()
    {
        var sent = new Sent();
        var wake = new Wake();
        var meeting = new MeetingShortcutModel(sent.Send);
        var recorder = new ShortcutRecorderModel(sent.Send, new DictationModel(sent.Send), _ => { }, wake, meeting);
        recorder.Start(ShortcutTarget.Meeting);
        wake.Run!();
        Assert.False(recorder.Busy);
        Assert.Equal(MeetingShortcutModel.DefaultKey, meeting.Key);
        Assert.False(sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last().Suspended);
        Assert.Contains("Couldn't pause", recorder.Message(ShortcutTarget.Meeting)!.Text, StringComparison.Ordinal);
    }

    [Fact]
    public void ACheckedChordDoesNotSaveOrResumeUntilTheRecordingPressIsReleased()
    {
        var sent = new Sent();
        var recorder = new ShortcutRecorderModel(sent.Send, new DictationModel(sent.Send), _ => { }, new Wake(), new MeetingShortcutModel(sent.Send));
        recorder.Start(ShortcutTarget.Meeting);
        var suspend = sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last();
        recorder.Apply(Ev.Of($$"""{"type":"meetings.shortcut.state","key":"ctrl+shift+r","active":false,"suspended":true,"ref":"{{suspend.Id}}"}"""));
        recorder.Feed(new ShortcutCapture.Input.KeyDown(CapturedKey.Side("left_control")));
        recorder.Feed(new ShortcutCapture.Input.KeyDown(CapturedKey.Of(0x4D)));
        var check = sent.Commands.OfType<CoreCommand.HotkeyCheck>().Last();
        recorder.Apply(Ev.Of($$"""{"type":"hotkey.checked","binding":"ctrl+m","ok":true,"canonical":"ctrl+m","ref":"{{check.Ref}}"}"""));
        Assert.Empty(sent.Commands.OfType<CoreCommand.SettingSet>());
        Assert.Single(sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>());
        recorder.Feed(new ShortcutCapture.Input.KeyUp(CapturedKey.Of(0x4D)));
        Assert.Empty(sent.Commands.OfType<CoreCommand.SettingSet>());
        recorder.Feed(new ShortcutCapture.Input.KeyUp(CapturedKey.Side("left_control")));
        Assert.Equal("ctrl+m", Assert.Single(sent.Commands.OfType<CoreCommand.SettingSet>()).Value);
        Assert.False(sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last().Suspended);
    }

    [Fact]
    public void AltGrCaptureFinishesOnTheReleaseWindowsActuallyReports()
    {
        var sent = new Sent();
        var recorder = new ShortcutRecorderModel(sent.Send, new DictationModel(sent.Send), _ => { }, new Wake(), new MeetingShortcutModel(sent.Send));
        recorder.Start(ShortcutTarget.Meeting);
        var suspend = sent.Commands.OfType<CoreCommand.MeetingsShortcutSuspend>().Last();
        recorder.Apply(Ev.Of($$"""{"type":"meetings.shortcut.state","key":"ctrl+shift+r","active":false,"suspended":true,"ref":"{{suspend.Id}}"}"""));
        recorder.Feed(new ShortcutCapture.Input.KeyDown(CapturedKey.Side("left_control")));
        recorder.Feed(new ShortcutCapture.Input.KeyDown(CapturedKey.Side("right_alt")));
        recorder.Feed(new ShortcutCapture.Input.KeyUp(CapturedKey.Side("left_control")));
        var check = sent.Commands.OfType<CoreCommand.HotkeyCheck>().Last();
        recorder.Apply(Ev.Of($$"""{"type":"hotkey.checked","binding":"right_alt","ok":true,"canonical":"right_alt","ref":"{{check.Ref}}"}"""));
        Assert.Equal("right_alt", Assert.Single(sent.Commands.OfType<CoreCommand.SettingSet>()).Value);
    }
}
