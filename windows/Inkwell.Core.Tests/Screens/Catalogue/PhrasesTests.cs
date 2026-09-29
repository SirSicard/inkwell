// Settings > Snippets and Voice commands, and the note about Inkwell 0.2's dictation key (as the
// Mac's PhrasesTests): what each model shows for the core's events and what it sends.
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

internal static class PhrasesSent
{
    public static IReadOnlyList<SnippetDraft>? LastSnippets(this Sent sent) =>
        sent.Commands.OfType<CoreCommand.SnippetsSave>().LastOrDefault()?.Snippets;

    public static CoreCommand.VoiceCommandsSave? LastCommands(this Sent sent) =>
        sent.Commands.OfType<CoreCommand.VoiceCommandsSave>().LastOrDefault();

    public const string ImportedSnippets = """{"type":"snippets.listed","from_import":true,"ref":"snippets:1","snippets":[{"id":"s1","trigger":"my sig","expansion":"Kind regards","category":"Email","enabled":true},{"id":"s2","trigger":"brb","expansion":"be right back","category":"","enabled":false}]}""";

    public const string ImportedCommands = """{"type":"voice_commands.listed","enabled":true,"wake_prefix":"inkwell","from_import":true,"commands":[{"id":"sig","triggers":["sign off"],"action":"insert_text","value":"Best, A.","enabled":true,"carried_out":true},{"id":"site","triggers":["open the site","site"],"action":"open_url","value":"https://example.com","enabled":true,"carried_out":false}]}""";

    public static string ImportedCommandsWithRef(string reference) =>
        ImportedCommands.Replace("\"from_import\":true", $"\"from_import\":true,\"ref\":\"{reference}\"", StringComparison.Ordinal);
}

public class SnippetsModelTests
{
    [Fact]
    public void TheListIsTheCoresAndAnImportSaysSo()
    {
        var sent = new Sent();
        var model = new SnippetsModel(sent.Send);
        model.Load();
        Assert.IsType<CoreCommand.SnippetsList>(Assert.Single(sent.Commands));
        Assert.False(model.Loaded);
        model.Apply(Ev.Of(PhrasesSent.ImportedSnippets));
        Assert.True(model.Loaded);
        Assert.True(model.FromImport);
        Assert.Equal(["my sig", "brb"], model.Rows.Select(r => r.Trigger));
        Assert.False(model.Rows[1].Enabled);
    }

    [Fact]
    public void AddEditToggleAndDeleteEachSendTheWholeList()
    {
        var sent = new Sent();
        var model = new SnippetsModel(sent.Send);
        model.Apply(Ev.Of(PhrasesSent.ImportedSnippets));

        model.Add("  ", "nothing", "");
        Assert.Null(sent.LastSnippets()); // a blank trigger adds nothing

        model.Add(" addr ", "1 Example Street", "Home");
        var saved = sent.LastSnippets() ?? [];
        Assert.Equal(3, saved.Count);
        Assert.Equal("addr", saved[^1].Trigger);
        Assert.Equal("Home", saved[^1].Category);
        Assert.Equal(3, saved.Select(s => s.Id).Distinct().Count()); // a new id

        model.Update(model.Rows[0] with { Expansion = "Best wishes" });
        Assert.Equal("Best wishes", sent.LastSnippets()?[0].Expansion);

        model.SetEnabled("s2", true);
        Assert.True(sent.LastSnippets()?[1].Enabled);

        model.Delete("s1");
        saved = sent.LastSnippets() ?? [];
        Assert.DoesNotContain("s1", saved.Select(s => s.Id));
        Assert.Equal(2, model.Rows.Count); // shown at once
    }

    /// <summary>Two quick changes: the answer to the first never shows over the second.</summary>
    [Fact]
    public void OnlyTheAnswerToTheNewestSaveReplacesTheRows()
    {
        var sent = new Sent();
        var model = new SnippetsModel(sent.Send);
        model.Load();
        model.Apply(Ev.Of(PhrasesSent.ImportedSnippets));
        model.Delete("s1");
        model.Delete("s2");
        Assert.Empty(model.Rows);
        // The first delete's answer (snippets:2) arrives after the second was sent.
        model.Apply(Ev.Of("""{"type":"snippets.listed","from_import":false,"ref":"snippets:2","snippets":[{"id":"s2","trigger":"brb","expansion":"be right back","category":"","enabled":false}]}"""));
        Assert.Empty(model.Rows); // an earlier answer is not shown
        model.Apply(Ev.Of("""{"type":"snippets.listed","from_import":false,"ref":"snippets:3","snippets":[]}"""));
        Assert.Empty(model.Rows);
        Assert.False(model.FromImport);
    }

    [Fact]
    public void AFailedSaveSaysSoAndReadsTheListAgain()
    {
        var sent = new Sent();
        var model = new SnippetsModel(sent.Send);
        model.Apply(Ev.Of(PhrasesSent.ImportedSnippets));
        model.Delete("s1");
        var before = sent.Commands.Count;
        model.Apply(Ev.Of("""{"type":"command.failed","command":"snippets.save","id":"snippets:2","message":"store failed"}"""));
        Assert.Equal(SnippetsModel.SaveFailedText, model.Failure);
        var reload = Assert.IsType<CoreCommand.SnippetsList>(sent.Commands.Skip(before).First());
        model.Apply(Ev.Of(PhrasesSent.ImportedSnippets.Replace("snippets:1", reload.Ref, StringComparison.Ordinal)));
        Assert.Null(model.Failure);
        Assert.Equal(2, model.Rows.Count); // the saved list, not the unsaved one
        model.Apply(Ev.Of("""{"type":"command.failed","command":"snippets.list","message":"the stored snippets cannot be read"}"""));
        Assert.Equal(SnippetsModel.LoadFailedText, model.Failure);
    }

    /// <summary>
    /// A list that could not be read (a damaged stored document) can't be changed: Add and every
    /// edit are inert, and only "Start over" replaces it, saying so to the core.
    /// </summary>
    [Fact]
    public void NothingChangesAListThatWasNotReadButStartOver()
    {
        var sent = new Sent();
        var model = new SnippetsModel(sent.Send);
        model.StartOver();
        model.Add("brb", "be right back", "");
        Assert.Empty(sent.Commands); // not loaded yet: Add is inert
        model.Load();
        model.Apply(Ev.Of("""{"type":"command.failed","command":"snippets.list","id":"snippets:1","message":"the stored snippets cannot be read"}"""));
        Assert.False(model.Loaded);
        Assert.True(model.Unreadable);
        Assert.Equal(SnippetsModel.LoadFailedText, model.Failure);
        model.Add("brb", "be right back", "");
        model.Update(new SnippetDraft("s1", "x", "y"));
        model.SetEnabled("s1", false);
        model.Delete("s1");
        Assert.Null(sent.LastSnippets()); // no save over a list that was not read
        model.StartOver();
        var save = Assert.IsType<CoreCommand.SnippetsSave>(sent.Commands[^1]);
        Assert.Empty(save.Snippets);
        Assert.True(save.ReplaceUnreadable);
        Assert.True(JsonDocument.Parse(save.Json).RootElement.GetProperty("replace_unreadable").GetBoolean());
        model.Apply(Ev.Of($$"""{"type":"snippets.listed","from_import":false,"ref":"{{save.Ref}}","snippets":[]}"""));
        Assert.True(model.Loaded);
        Assert.False(model.Unreadable);
        Assert.Null(model.Failure);
        Assert.False(JsonDocument.Parse(new CoreCommand.SnippetsSave([], false, "r").Json).RootElement.TryGetProperty("replace_unreadable", out _)); // sent only when chosen
    }

    /// <summary>
    /// The stored list became unreadable between reading it and saving: the core refuses the save,
    /// and Start over appears at once, not only a generic "couldn't save".
    /// </summary>
    [Fact]
    public void ASaveRefusedOverAListThatBecameUnreadableOffersStartOver()
    {
        var sent = new Sent();
        var model = new SnippetsModel(sent.Send);
        model.Load();
        model.Apply(Ev.Of(PhrasesSent.ImportedSnippets));
        model.Delete("s1");
        model.Apply(Ev.Of("""{"type":"command.failed","command":"snippets.save","id":"snippets:2","code":"list_unreadable","message":"the stored snippets cannot be read, so a save would replace them; send replace_unreadable to start over"}"""));
        Assert.True(model.Unreadable);
        Assert.False(model.Loaded);
        Assert.Empty(model.Rows);
        Assert.Equal(SnippetsModel.LoadFailedText, model.Failure);
        Assert.IsType<CoreCommand.SnippetsList>(sent.Commands[^1]);
        model.StartOver();
        Assert.True(Assert.IsType<CoreCommand.SnippetsSave>(sent.Commands[^1]).ReplaceUnreadable);

        var commands = new VoiceCommandsModel(sent.Send);
        commands.Load();
        commands.Apply(Ev.Of(PhrasesSent.ImportedCommandsWithRef("voice_commands:1")));
        commands.SetEnabled(false);
        commands.Apply(Ev.Of("""{"type":"command.failed","command":"voice_commands.save","id":"voice_commands:2","code":"list_unreadable","message":"the stored voice commands cannot be read, so a save would replace them; send replace_unreadable to start over"}"""));
        Assert.True(commands.Unreadable);
        commands.StartOver();
        Assert.True(Assert.IsType<CoreCommand.VoiceCommandsSave>(sent.Commands[^1]).ReplaceUnreadable);
    }

    /// <summary>The refusal is told apart by its code: a reworded message still offers Start over.</summary>
    [Fact]
    public void ARefusalIsKnownByItsCodeNotItsMessage()
    {
        var sent = new Sent();
        var model = new SnippetsModel(sent.Send);
        model.Load();
        model.Apply(Ev.Of(PhrasesSent.ImportedSnippets));
        model.Delete("s1");
        model.Apply(Ev.Of("""{"type":"command.failed","command":"snippets.save","id":"snippets:2","code":"list_unreadable","message":"reworded"}"""));
        Assert.True(model.Unreadable);
        Assert.Equal(SnippetsModel.LoadFailedText, model.Failure);

        var commands = new VoiceCommandsModel(sent.Send);
        commands.Load();
        commands.Apply(Ev.Of(PhrasesSent.ImportedCommandsWithRef("voice_commands:1")));
        commands.SetEnabled(false);
        commands.Apply(Ev.Of("""{"type":"command.failed","command":"voice_commands.save","id":"voice_commands:2","code":"list_unreadable","message":"reworded"}"""));
        Assert.True(commands.Unreadable);
        Assert.Equal(VoiceCommandsModel.LoadFailedText, commands.Failure);
    }

    [Fact]
    public void TheSaveCommandCarriesEveryField()
    {
        var json = new CoreCommand.SnippetsSave([new SnippetDraft("a", "brb", "be right back", "", Enabled: false)], false, "snippets:9").Json;
        var root = JsonDocument.Parse(json).RootElement;
        Assert.Equal("snippets.save", root.GetProperty("cmd").GetString());
        Assert.Equal("snippets:9", root.GetProperty("id").GetString());
        var first = root.GetProperty("snippets")[0];
        Assert.Equal("brb", first.GetProperty("trigger").GetString());
        Assert.False(first.GetProperty("enabled").GetBoolean());
    }
}

public class VoiceCommandsModelTests
{
    [Fact]
    public void ImportedCommandsAreListedWithWhatThisBuildDoes()
    {
        var model = new VoiceCommandsModel(_ => { });
        model.Apply(Ev.Of(PhrasesSent.ImportedCommands));
        Assert.True(model.Enabled);
        Assert.True(model.FromImport);
        Assert.Equal([true, false], model.Rows.Select(r => r.CarriedOut));
        Assert.Equal("Type “Best, A.”", VoiceCommandsModel.Describe(model.Rows[0]));
        Assert.Equal("Open https://example.com", VoiceCommandsModel.Describe(model.Rows[1]));
        Assert.Equal("Say “inkwell”, then a command", model.StatusLine);
    }

    [Fact]
    public void ChangesSendTheWholeStore()
    {
        var sent = new Sent();
        var model = new VoiceCommandsModel(sent.Send);
        model.Apply(Ev.Of(PhrasesSent.ImportedCommands));
        model.SetEnabled(false);
        Assert.False(sent.LastCommands()?.Enabled);
        Assert.Equal(2, sent.LastCommands()?.Commands.Count); // the commands go with the switch
        Assert.Equal("Off: everything you say is dictated", model.StatusLine);

        model.SetWakePrefix("   ");
        Assert.Equal("inkwell", sent.LastCommands()?.WakePrefix); // a blank wake word changes nothing
        Assert.False(model.CanSaveWakePrefix(" Inkwell "));
        model.SetWakePrefix(" Computer ");
        Assert.Equal("computer", sent.LastCommands()?.WakePrefix);

        model.SetCommandEnabled("site", false);
        Assert.False(sent.LastCommands()?.Commands[1].Enabled);

        model.Add("Sign Here, , sig", CommandAction.InsertText, "A. Writer");
        Assert.Equal(["sign here", "sig"], sent.LastCommands()?.Commands[^1].Triggers ?? []);
        Assert.Equal("A. Writer", sent.LastCommands()?.Commands[^1].Value);
        var count = sent.Commands.Count;
        model.Add("x", CommandAction.OpenUrl, "https://example.com");
        model.Add(" , ", CommandAction.InsertText, "y");
        Assert.Equal(count, sent.Commands.Count); // only kinds this build does, and never without a phrase

        model.Delete("sig");
        Assert.DoesNotContain("sig", sent.LastCommands()?.Commands.Select(c => c.Id) ?? []);
    }

    [Fact]
    public void TheSaveCommandCarriesTheActionAndItsValue()
    {
        var json = new CoreCommand.VoiceCommandsSave(
            true, "inkwell",
            [
                new VoiceCommandDraft("u", ["scratch that"], CommandAction.Undo, null),
                new VoiceCommandDraft("t", ["sign off"], CommandAction.InsertText, "Best"),
            ],
            false, "voice_commands:3").Json;
        var root = JsonDocument.Parse(json).RootElement;
        var commands = root.GetProperty("commands");
        Assert.Equal("undo", commands[0].GetProperty("action").GetString());
        Assert.False(commands[0].TryGetProperty("value", out _));
        Assert.Equal("Best", commands[1].GetProperty("value").GetString());
        Assert.Equal("inkwell", root.GetProperty("wake_prefix").GetString());
    }

    [Fact]
    public void NothingChangesCommandsThatWereNotReadButStartOver()
    {
        var sent = new Sent();
        var model = new VoiceCommandsModel(sent.Send);
        model.SetEnabled(true);
        model.Add("sign off", CommandAction.InsertText, "Best");
        Assert.Empty(sent.Commands); // not loaded yet: inert
        model.Load();
        model.Apply(Ev.Of("""{"type":"command.failed","command":"voice_commands.list","id":"voice_commands:1","message":"the stored voice commands cannot be read"}"""));
        Assert.True(model.Unreadable);
        model.SetEnabled(true);
        model.SetWakePrefix("computer");
        model.Add("sign off", CommandAction.InsertText, "Best");
        Assert.Null(sent.LastCommands());
        model.StartOver();
        var save = Assert.IsType<CoreCommand.VoiceCommandsSave>(sent.Commands[^1]);
        Assert.False(save.Enabled);
        Assert.Empty(save.Commands);
        Assert.True(save.ReplaceUnreadable);
    }

    [Fact]
    public void AFailedSaveSaysSoAndReadsAgain()
    {
        var sent = new Sent();
        var model = new VoiceCommandsModel(sent.Send);
        model.Apply(Ev.Of(PhrasesSent.ImportedCommands));
        model.SetEnabled(false);
        model.Apply(Ev.Of("""{"type":"command.failed","command":"voice_commands.save","id":"voice_commands:1","message":"store failed"}"""));
        Assert.Equal(VoiceCommandsModel.SaveFailedText, model.Failure);
        Assert.IsType<CoreCommand.VoiceCommandsList>(sent.Commands[^1]);
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"voice_commands.save","id":"voice_commands:1","message":"x"}""");
        // Windows: the model says it shows it (the Mac asked ScreenModels.handles).
        Assert.True(VoiceCommandsModel.Handles(failed)); // the screen shows it
        Assert.True(SnippetsModel.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"snippets.list","message":"x"}""")));
    }
}

public class ImportNoteTests
{
    [Fact]
    public void AnUnmappableKeyIsSaidUntilDismissed()
    {
        var sent = new Sent();
        var model = new ImportNoteModel(sent.Send);
        model.Load();
        Assert.Equal([new CoreCommand.ImportNotes()], sent.Commands);
        model.Apply(Ev.Of("""{"type":"import.notes","key":{"hotkey":"super+shift+space","outcome":"combination","applied":false,"toggle":false}}"""));
        var note = Assert.IsType<ImportKeyNote>(model.Note);
        // Windows: Win+Shift+Space and Right Ctrl, where the Mac shows its symbols and fn (Globe).
        Assert.Equal(
            "Inkwell 0.2 started dictation with Win+Shift+Space, a key combination. Inkwell now listens for one key held on its own, so it uses Right Ctrl. Pick another under Dictate if you like.",
            ImportNoteModel.Text(note, "Right Ctrl"));
        model.Dismiss();
        Assert.Null(model.Note);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.ImportKeyNote, "dismissed"), sent.Commands[^1]);
        model.Apply(Ev.Of("""{"type":"import.notes"}"""));
        Assert.Null(model.Note); // nothing to say
    }

    [Fact]
    public void AToggleKeyThatCarriedOverSaysItIsNowHeld()
    {
        var model = new ImportNoteModel(_ => { });
        model.Apply(Ev.Of("""{"type":"import.notes","key":{"hotkey":"right_ctrl","outcome":"mapped","key":"right_control","applied":true,"toggle":true}}"""));
        var text = ImportNoteModel.Text(Assert.IsType<ImportKeyNote>(model.Note), "Right Ctrl");
        Assert.StartsWith("Inkwell 0.2 started and stopped on separate presses.", text, StringComparison.Ordinal);
        Assert.Contains("hold Right Ctrl", text, StringComparison.Ordinal);
    }

    /// <summary>Windows: 0.2's fn was replaced by right Ctrl; the note names both keys.</summary>
    [Fact]
    public void AReplacedKeyNamesBoth()
    {
        var model = new ImportNoteModel(_ => { });
        model.Apply(Ev.Of("""{"type":"import.notes","key":{"hotkey":"fn","outcome":"replaced","key":"right_control","applied":true,"toggle":false}}"""));
        Assert.Equal(
            "Inkwell 0.2 started dictation with fn, which never reaches Windows, so it was replaced by Right Ctrl. Pick another under Dictate if you like.",
            ImportNoteModel.Text(Assert.IsType<ImportKeyNote>(model.Note), "Right Ctrl"));
    }

    /// <summary>Windows: keys by their Windows names (the Mac shows its modifier symbols).</summary>
    [Fact]
    public void OldKeysReadAsKeys()
    {
        Assert.Equal("Ctrl+Space", ImportNoteModel.Keys("ctrl+space"));
        Assert.Equal("Alt+F13", ImportNoteModel.Keys("alt+f13"));
        Assert.Equal("Right Windows key", ImportNoteModel.Keys("right_cmd"));
        Assert.Equal("F13", ImportNoteModel.Keys("f13"));
        Assert.Equal("ctrl++", ImportNoteModel.Keys("ctrl++")); // not a key combination this can read: as stored
    }
}
