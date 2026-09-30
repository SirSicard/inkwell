// The commands are the JSON the core reads (as the Mac's CoreCommandTests), and their logged name
// never carries a field.
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class CoreCommandTests
{
    private static JsonElement Fields(CoreCommand c) => JsonDocument.Parse(c.Json).RootElement;

    [Fact]
    public void CommandsAreTheJsonTheCoreReads()
    {
        var add = Fields(new CoreCommand.NoteAdd("r", 754_000, "Pilot", "note-line-0"));
        Assert.Equal("note.add", add.GetProperty("cmd").GetString());
        Assert.Equal(JsonValueKind.Number, add.GetProperty("at_ms").ValueKind);
        Assert.Equal(754_000, add.GetProperty("at_ms").GetInt64());
        Assert.Equal("note-line-0", add.GetProperty("id").GetString());
        Assert.True(Fields(new CoreCommand.CommitmentSetDone("c", true)).GetProperty("done").GetBoolean());
        Assert.Equal("system_audio", Fields(new CoreCommand.PermissionRequest(PermissionName.SystemAudio)).GetProperty("permission").GetString());
        Assert.Equal("live_partials", Fields(new CoreCommand.EngineRoute(Job.LivePartials)).GetProperty("job").GetString());
        var warm = Fields(new CoreCommand.ModelWarm(Job.DictationFinal));
        Assert.Equal("model.warm", warm.GetProperty("cmd").GetString());
        Assert.Equal("dictation_final", warm.GetProperty("job").GetString());
        Assert.Equal("model.warm:dictation_final", warm.GetProperty("id").GetString());
        Assert.Equal("model.warm", new CoreCommand.ModelWarm(Job.DictationFinal).Name);
        Assert.Equal("note.add", new CoreCommand.NoteAdd("r", 1, "private words", "x").Name);
        Assert.DoesNotContain("private", new CoreCommand.NoteAdd("r", 1, "private words", "x").Name);
    }

    [Fact]
    public void ASettingCommandCarriesItsKeyAsTheId()
    {
        var set = Fields(new CoreCommand.SettingSet(ShellSetting.DictationEditKey, "off"));
        Assert.Equal("dictation.edit_key", set.GetProperty("key").GetString());
        Assert.Equal("setting:dictation.edit_key", set.GetProperty("id").GetString());
        Assert.Equal("off", set.GetProperty("value").GetString());
    }

    [Fact]
    public void OptionalFieldsAreLeftOutAndNestedOnesAreObjects()
    {
        var first = Fields(new CoreCommand.RecordsList(null, null, 50, "q1"));
        Assert.False(first.TryGetProperty("kind", out _));
        Assert.False(first.TryGetProperty("before", out _));
        var next = Fields(new CoreCommand.RecordsList(RecordKind.FileImport, new RecordCursor(42, "r9"), 50, "q2"));
        Assert.Equal("file_import", next.GetProperty("kind").GetString());
        Assert.Equal(42, next.GetProperty("before").GetProperty("started_at_unix_ms").GetInt64());
        Assert.Equal("r9", next.GetProperty("before").GetProperty("id").GetString());

        var allow = Fields(new CoreCommand.ConsentAllow(LlmFeature.Edit, LlmDestination.Cloud, "shell engine cloud", "right_alt", "c1"));
        Assert.Equal("edit", allow.GetProperty("feature").GetString());
        Assert.Equal("cloud", allow.GetProperty("to").GetString());
        Assert.Equal("right_alt", allow.GetProperty("key").GetString());
        Assert.False(Fields(new CoreCommand.ConsentAllow(LlmFeature.Polish, LlmDestination.OnDevice, null, null, "c2")).TryGetProperty("endpoint", out _));
    }

    [Fact]
    public void ListsSaveWholeAndCompareByContent()
    {
        var save = new CoreCommand.VoiceCommandsSave(true, "computer",
            [new VoiceCommandDraft("v1", ["new line"], CommandAction.InsertText, "\n")], false, "s1");
        var json = Fields(save);
        Assert.Equal("new line", json.GetProperty("commands")[0].GetProperty("triggers")[0].GetString());
        Assert.Equal("\n", json.GetProperty("commands")[0].GetProperty("value").GetString());
        Assert.False(json.TryGetProperty("replace_unreadable", out _));
        Assert.Equal(save, new CoreCommand.VoiceCommandsSave(true, "computer",
            [new VoiceCommandDraft("v1", new List<string> { "new line" }, CommandAction.InsertText, "\n")], false, "s1"));

        var snippets = new CoreCommand.SnippetsSave([new SnippetDraft("s", "sig", "Best,")], true, "s2");
        Assert.True(Fields(snippets).GetProperty("replace_unreadable").GetBoolean());
        Assert.Equal(snippets, new CoreCommand.SnippetsSave(new List<SnippetDraft> { new("s", "sig", "Best,") }, true, "s2"));
    }

    [Fact]
    public void ACommandThatNeverReachedTheCoreFailsWithItsNameAndId()
    {
        var ask = new CoreCommand.MeetingAsk("a private question", "ask:3");
        var failed = ask.NotSent("couldn't send it: the core is not running");
        Assert.Equal("command.failed", failed.Type);
        Assert.Equal("meeting.ask", failed.Command);
        Assert.Equal("ask:3", failed.Id);
        Assert.DoesNotContain("private", failed.Message);
        Assert.Null(new CoreCommand.ModesList().NotSent("x").Id);
        Assert.Equal("engine.route:live_partials", new CoreCommand.EngineRoute(Job.LivePartials).CommandId);
    }
}
