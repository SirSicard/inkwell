// The aggregator's wiring (as the Mac's ScreenModels and CoreControllerCommandTests): what the
// screens read when the core is ready, which failures a screen shows, and that the rest are logged
// by command name only.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class ScreenModelsTests
{
    private static InkEvent Ready => Ev.Of($$"""{"type":"core.ready","abi":{{InkSession.AbiVersion}},"version":"1.0.0"}""");

    private static CommandFailed Failed(string command, string? id = null) => Ev.Of<CommandFailed>(
        id is null
            ? $$"""{"type":"command.failed","command":"{{command}}","message":"the library could not be written: zebra"}"""
            : $$"""{"type":"command.failed","command":"{{command}}","id":"{{id}}","message":"the library could not be written: zebra"}""");

    [Fact]
    public void TheCoreBeingReadyReadsWhatTheFirstScreensNeed()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send, log: new Logged().Log);
        screens.Apply([Ready]);
        var names = sent.Commands.Select(c => c.Name).ToList();
        Assert.Contains(new CoreCommand.SettingGet(ShellSetting.OnboardingDone), sent.Commands);
        Assert.Contains("permissions.check", names);
        Assert.Contains("models.list", names);
        Assert.Contains(new CoreCommand.SettingGet(ShellSetting.MeetingsDetect), sent.Commands);
        Assert.Contains(sent.Commands, c => c is CoreCommand.ConsentGet { Feature: LlmFeature.Polish });
        Assert.Contains(sent.Commands, c => c is CoreCommand.ConsentGet { Feature: LlmFeature.Edit });
        Assert.Contains(sent.Commands, c => c is CoreCommand.ConsentGet { Feature: LlmFeature.Meetings });
        Assert.Contains(new CoreCommand.SettingGet(ShellSetting.DictationEnabled), sent.Commands);
        Assert.Contains(sent.Commands, c => c is CoreCommand.RecordsList);
    }

    [Fact]
    public void AFailureNoScreenHandlesIsLoggedByNameOnly()
    {
        var logged = new Logged();
        var screens = new ScreenModels(new Sent().Send, log: logged.Log);
        screens.LogUnshown([Failed("setting.set", "setting:onboarding.done")]);
        var line = Assert.Single(logged.Messages);
        Assert.Contains("setting.set", line);
        Assert.DoesNotContain("zebra", line);
        Assert.DoesNotContain("onboarding.done", line);

        // Handled ones are the screens' to show.
        foreach (var (command, id) in new (string, string?)[]
        {
            ("permissions.check", null), ("models.list", null), ("modes.list", null), ("commitment.set_done", null),
            ("note.add", "r:line:0"), ("note.update", "r:line:0:update"), ("note.delete", "r:line:0:delete"),
            ("setting.get", "setting:dictation.polish"), ("setting.set", "setting:dictation.key"),
            ("setting.set", "setting:meetings.llm"), ("setting.get", "setting:meetings.detect"),
            ("consent.allow", "consent.allow:edit:3"), ("snippets.save", "snippets:2"), ("meeting.start", "meeting.start"),
        })
        {
            Assert.True(screens.Handles(Failed(command, id)), $"{command} {id}");
        }
        screens.LogUnshown([Failed("setting.get", "setting:somebody.else")]);
        Assert.Equal(2, logged.Messages.Count);
    }

    [Fact]
    public void ANoteStillUnderTheCaretIsHandedOverBeforeTheCoreStops()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send, log: new Logged().Log);
        screens.Apply([Ev.Of("""{"type":"meeting.started","record":"r1"}""")]);
        screens.Live.NotesEdited("A line still being typed", 0);
        Assert.DoesNotContain(sent.Commands, c => c is CoreCommand.NoteAdd);
        screens.FlushBeforeStop();
        var add = Assert.IsType<CoreCommand.NoteAdd>(Assert.Single(sent.Commands, c => c is CoreCommand.NoteAdd));
        Assert.Equal("A line still being typed", add.Text);
    }

    /// <summary>A command that never reached the core resolves its screen to "couldn't", as the core's own failure does.</summary>
    [Fact]
    public void ACommandNeverSentResolvesItsScreenInsteadOfWaiting()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send, log: new Logged().Log);
        screens.Library.RefreshList();
        Assert.Equal(LibraryLoad.Loading, screens.Library.ListLoad);
        var list = Assert.Single(sent.Commands, c => c is CoreCommand.RecordsList);
        var failed = list.NotSent("couldn't send it: the core is not running");
        Assert.True(screens.Handles(failed));
        screens.Apply([failed]);
        Assert.Equal(LibraryLoad.Failed, screens.Library.ListLoad);
    }
}
