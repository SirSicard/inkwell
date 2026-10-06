// Settings > Modes' editor (as the Mac's ModesEditorTests): what each row and the editor say for the
// core's listing (the chips, a mode's own model), what each save, delete, confirm and OK sends,
// every refusal's words, and the same against the real core.
using System.Text.Json;
using System.Text.Json.Nodes;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

internal static class ModesJson
{
    public const string Apple = """{"id":"engine:local-llm","name":"Example Local Model","model":"example-local","to":"on_device","allowed":true,"blocked_local_only":false}""";

    public static string Groq(bool allowed, bool blocked = false) =>
        $$"""{"id":"provider:groq","name":"Groq","model":"llama-3.1-8b-instant","to":"cloud","endpoint":"https://api.groq.com/openai/v1","allowed":{{B(allowed)}},"blocked_local_only":{{B(blocked)}}}""";

    public const string GroqConsent = """{"to":"cloud","name":"Groq","endpoint":"https://api.groq.com/openai/v1"}""";
    public const string DeviceConsent = """{"to":"on_device"}""";

    public static string Mode(
        string id, string name, string style = "casual", bool polish = true, bool fillers = true, string prompt = "",
        string[]? apps = null, string? model = null, string? modelName = null, string? state = null)
    {
        var fields = $$"""
            "id":"{{id}}","name":"{{name}}","style":"{{style}}","polish":{{B(polish)}},"remove_fillers":{{B(fillers)}},"polish_prompt":"{{prompt}}","apps":{{JsonSerializer.Serialize(apps ?? [])}}
            """;
        if (model is not null)
        {
            fields += $",\"polish_model\":\"{model}\"";
        }
        if (modelName is not null)
        {
            fields += $",\"polish_model_name\":\"{modelName}\"";
        }
        if (state is not null)
        {
            fields += $",\"polish_model_state\":\"{state}\"";
        }
        return "{" + fields + "}";
    }

    public static readonly string Default = Mode("d", "Default", style: "formal", polish: false);

    public static InkEvent Listing(IEnumerable<string> modes, IEnumerable<string>? models = null, string? setting = "engine:local-llm", string? reference = null, string? saved = null)
    {
        var fields = $$"""
            "type":"modes.listed","default_id":"d","default_polish_prompt":"Tidy it.","modes":[{{string.Join(",", modes)}}],"polish_models":[{{string.Join(",", models ?? [Apple])}}]
            """;
        if (setting is not null)
        {
            fields += $",\"setting_polish_model\":\"{setting}\"";
        }
        if (reference is not null)
        {
            fields += $",\"ref\":\"{reference}\"";
        }
        if (saved is not null)
        {
            fields += $",\"saved\":\"{saved}\"";
        }
        return Ev.Of("{" + fields + "}");
    }

    public static InkEvent PolishState(bool on, IEnumerable<string>? consents = null, string? reference = null)
    {
        var fields = $$"""
            "type":"consent.state","feature":"polish","on":{{B(on)}},"allowed":true,"to":"on_device","name":"Example Local Model","consents":[{{string.Join(",", consents ?? [])}}]
            """;
        if (reference is not null)
        {
            fields += $",\"ref\":\"{reference}\"";
        }
        return Ev.Of("{" + fields + "}");
    }

    public static InkEvent Failed(string command, string id, string? code) =>
        Ev.Of($$"""{"type":"command.failed","command":"{{command}}","id":"{{id}}","message":"refused: the mode's name zebra"{{(code is null ? "" : $",\"code\":\"{code}\"")}}}""");

    /// <summary>The modes.save commands sent, as JSON.</summary>
    public static List<JsonObject> Saves(Sent sent) =>
        sent.Commands.OfType<CoreCommand.ModesSave>().Select(c => JsonNode.Parse(c.Json)!.AsObject()).ToList();

    public static JsonObject ModeOf(JsonObject save) => save["mode"]!.AsObject();

    private static string B(bool b) => b ? "true" : "false";
}

internal sealed class FakeRunning(params PickedApp[] apps) : IRunningApps
{
    public IReadOnlyList<PickedApp> Running() => apps;
}

public class ModesRowsTests
{
    private static (ModesModel, Sent) Model(bool? switchOn = true)
    {
        var sent = new Sent();
        return (new ModesModel(sent.Send, polishSwitch: () => switchOn), sent);
    }

    /// <summary>The "Polish" chip shows only when polish runs; with no model at all it does not show, and with one that can't be used now it is "Polish · off", saying why.</summary>
    [Fact]
    public void ThePolishChipShowsOnlyWithAModel()
    {
        var (none, _) = Model();
        none.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat"), ModesJson.Default], models: [], setting: null));
        Assert.IsType<PolishState.NoModel>(none.Rows[0].Polish);
        Assert.Null(none.Rows[0].Polish.Chip);
        Assert.Null(none.Rows[0].Polish.RowNote);

        var (ready, _) = Model();
        ready.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat"), ModesJson.Mode("q", "Quiet", polish: false), ModesJson.Default]));
        Assert.Equal("Polish", ready.Rows[0].Polish.Chip);
        Assert.False(ready.Rows[0].Polish.ChipDimmed);
        Assert.Null(ready.Rows[1].Polish.Chip);
        Assert.Equal(["Casual", "Clean up speech"], ready.Rows[0].Traits);
        Assert.Equal(["Casual", "Clean up speech", "Polish"], ready.Rows[0].Chips);
        Assert.Equal(["Chat", "Quiet", "Everywhere else"], ready.Rows.Select(r => r.Title));
        Assert.Equal("Default", ready.Rows[^1].Name);

        var (off, _) = Model(switchOn: false);
        off.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat"), ModesJson.Default]));
        Assert.IsType<PolishState.SwitchedOff>(off.Rows[0].Polish);
        Assert.Equal("Polish · off", off.Rows[0].Polish.Chip);
        Assert.True(off.Rows[0].Polish.ChipDimmed);
        Assert.Equal("Polish my words is off in AI", off.Rows[0].Polish.Why);
        Assert.Equal("Polish, off: Polish my words is off in AI", off.Rows[0].Polish.SpokenChip);
        Assert.Contains("Polish, off: Polish my words is off in AI", off.Rows[0].AccessibilityLabel, StringComparison.Ordinal);

        var (paused, _) = Model();
        paused.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat"), ModesJson.Default], models: [ModesJson.Groq(false)], setting: "provider:groq"));
        Assert.Equal("Polish needs your OK again in AI", paused.Rows[0].Polish.Why);
        Assert.Null(paused.Rows[0].Polish.RowNote);

        var (local, _) = Model();
        local.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat", model: "provider:groq", state: "ready"), ModesJson.Default],
            models: [ModesJson.Apple, ModesJson.Groq(true, blocked: true)]));
        Assert.Equal("Local only is on in AI", local.Rows[0].Polish.Why);
        Assert.Equal("Local only is on, so nothing goes to Groq · llama-3.1-8b-instant. Turn Local only off in AI to use it.", local.Rows[0].Polish.RowNote?.Text);
    }

    /// <summary>A mode's own model: missing shows no chip and says so; moved and never recorded offer Confirm…; one no consent covers offers Allow….</summary>
    [Fact]
    public void AModesOwnModelSaysWhatStopsIt()
    {
        var (modes, _) = Model();
        modes.Apply(ModesJson.Listing([
            ModesJson.Mode("m", "Missing", model: "provider:openai", modelName: "gpt-x", state: "missing"),
            ModesJson.Mode("v", "Moved", model: "provider:groq", state: "moved"),
            ModesJson.Mode("u", "Unrecorded", model: "provider:groq", modelName: "llama-big", state: "unrecorded"),
            ModesJson.Mode("k", "NeedsOK", model: "provider:groq", state: "ready"),
            ModesJson.Mode("r", "Ready", model: "engine:local-llm", state: "ready"),
            ModesJson.Mode("x", "Gone", model: "engine:someone-else", state: "missing"),
            ModesJson.Default,
        ], models: [ModesJson.Apple, ModesJson.Groq(false)]));
        var byName = modes.Rows.ToDictionary(r => r.Name, r => r.Polish);

        Assert.Null(byName["Missing"].Chip);
        Assert.Equal("Its model, OpenAI · gpt-x, isn't available now, so this mode isn't polished. Edit it to pick another.", byName["Missing"].RowNote?.Text);
        Assert.Null(byName["Missing"].RowNote?.Fix);
        Assert.Equal("Its model, a model that was on this PC, isn't available now, so this mode isn't polished. Edit it to pick another.", byName["Gone"].RowNote?.Text);

        Assert.Equal("Polish · off", byName["Moved"].Chip);
        Assert.Equal("Confirm where its model sends", byName["Moved"].Why);
        Assert.Equal("Its model now sends somewhere else: Groq · llama-3.1-8b-instant. Confirm it to polish with it again.", byName["Moved"].RowNote?.Text);
        Assert.Equal(PolishFix.Confirm, byName["Moved"].RowNote?.Fix);
        Assert.Equal("Where its model sends was never recorded: Groq · llama-big. Confirm it to polish with it.", byName["Unrecorded"].RowNote?.Text);

        Assert.Equal("Polish needs your OK to send to Groq", byName["NeedsOK"].Why);
        Assert.Equal("Polish needs your OK to send this mode's words to Groq.", byName["NeedsOK"].RowNote?.Text);
        Assert.Equal(PolishFix.Allow, byName["NeedsOK"].RowNote?.Fix);

        Assert.Equal("Polish", byName["Ready"].Chip);
        Assert.Null(byName["Ready"].RowNote);
        foreach (var row in modes.Rows)
        {
            foreach (var text in new[] { row.Polish.Chip, row.Polish.Why, row.Polish.RowNote?.Text, row.AccessibilityLabel }.OfType<string>())
            {
                Assert.DoesNotContain("provider:", text, StringComparison.Ordinal);
                Assert.DoesNotContain("engine:", text, StringComparison.Ordinal);
            }
        }
    }

    /// <summary>Verify: a take whose mode's model is missing reads the modes again, to show which (the Drop says so: DropModelTests).</summary>
    [Fact]
    public void ATakeWhoseModesModelIsMissingReadsTheModesAgain()
    {
        var (modes, sent) = Model();
        modes.Load();
        sent.Commands.Clear();
        modes.Apply(Ev.Of("""{"type":"dictation.warning","kind":"polish_model_missing"}"""));
        Assert.Equal([new CoreCommand.ModesList("modes:2")], sent.Commands);
    }

    [Fact]
    public void AChangeElsewhereReadsTheModesAgainOnlyOnceRead()
    {
        var (modes, sent) = Model();
        modes.Apply(ModesJson.PolishState(true));
        Assert.Empty(sent.Commands);
        modes.Load();
        modes.Apply(ModesJson.PolishState(true));
        modes.Apply(Ev.Of("""{"type":"setting.value","key":"llm.local_only","value":"on"}"""));
        modes.Apply(Ev.Of("""{"type":"engine.unregistered","id":"local-llm"}"""));
        Assert.Equal(
            [new CoreCommand.ModesList("modes:1"), new CoreCommand.ModesList("modes:2"), new CoreCommand.ModesList("modes:3"), new CoreCommand.ModesList("modes:4")],
            sent.Commands);
    }

    [Fact]
    public void DeletingSaysWhereItsAppsGo()
    {
        var (modes, sent) = Model();
        modes.Apply(ModesJson.Listing([
            ModesJson.Mode("c", "Chat", apps: ["outlook.exe", "slack.exe"]), ModesJson.Mode("n", "Notes", apps: ["onenote.exe"]),
            ModesJson.Mode("e", "Empty"), ModesJson.Default]));
        Assert.Equal("Outlook and Slack go back to Everywhere else.", ModesModel.DeleteMessage(modes.Rows[0]));
        Assert.Equal("OneNote goes back to Everywhere else.", ModesModel.DeleteMessage(modes.Rows[1]));
        Assert.Equal("It has no apps. Voice commands can't switch to it any more.", ModesModel.DeleteMessage(modes.Rows[2]));
        Assert.Equal("Delete “Chat”?", ModesModel.DeleteTitle(modes.Rows[0]));
        modes.AskDelete("d");
        Assert.Null(modes.Deleting); // the default mode can't be deleted
        modes.AskDelete("c");
        Assert.Equal("Chat", modes.Deleting?.Name);
        modes.Delete(modes.Deleting!);
        Assert.Equal([new CoreCommand.ModesDelete("c", "modes:1")], sent.Commands);
        Assert.Null(modes.Deleting);
        modes.Apply(ModesJson.Failed("modes.delete", "modes:1", "mode_not_found"));
        Assert.Equal("“Chat” was already deleted.", modes.Problem);
    }

    /// <summary>The modes could not be read: nothing can be changed, and Start over (which asks first) replaces them.</summary>
    [Fact]
    public void UnreadableModesOfferStartOver()
    {
        var (modes, sent) = Model();
        modes.Load();
        modes.Apply(ModesJson.Failed("modes.list", "modes:1", null));
        Assert.True(modes.Failed);
        Assert.True(modes.Unreadable);
        modes.Add();
        Assert.Null(modes.Editor);
        modes.StartOver();
        Assert.Empty(ModesJson.Saves(sent)); // not without asking
        modes.AskStartOver();
        Assert.True(modes.ConfirmingStartOver);
        modes.StartOver();
        var save = Assert.Single(ModesJson.Saves(sent));
        Assert.True(save["replace_unreadable"]!.GetValue<bool>());
        Assert.Equal("default", ModesJson.ModeOf(save)["id"]!.GetValue<string>());
        Assert.Single(ModesJson.ModeOf(save)); // it changes nothing else
        modes.Apply(ModesJson.Listing([ModesJson.Default], reference: "modes:2", saved: "d"));
        Assert.False(modes.Failed);
        Assert.False(modes.Unreadable);
        Assert.False(modes.Busy);
    }
}

public class ModesEditorModelTests
{
    private static readonly string[] SlackApp = ["slack.exe"];

    private static (ModesModel Modes, Sent Sent, ConsentModel Consent) Model(
        IEnumerable<string>? modes = null, IEnumerable<string>? models = null, string? setting = "engine:local-llm",
        bool? switchOn = true, params PickedApp[] running)
    {
        var sent = new Sent();
        var consent = new ConsentModel(LlmFeature.Polish, PolishModel.SettingId, sent.Send);
        var model = new ModesModel(sent.Send, running: new FakeRunning(running), consent: consent, polishSwitch: () => switchOn);
        model.Apply(ModesJson.Listing(modes ?? [ModesJson.Mode("c", "Chat", apps: SlackApp), ModesJson.Default], models, setting));
        return (model, sent, consent);
    }

    [Fact]
    public void AddingSendsEveryFieldAndTheAnswerClosesTheEditor()
    {
        var (modes, sent, _) = Model();
        modes.Add();
        var editor = modes.Editor!;
        Assert.True(editor.Adding);
        editor.Name = "Email";
        editor.Style = ModeStyle.Relaxed;
        editor.Polish = true;
        // A TextBox's line break, as typed: the core judges whether it is the default.
        editor.Prompt = "Keep it short.\rNo emoji.";
        ModesModel.AddApp("outlook.exe", editor);
        ModesModel.AddApp("OUTLOOK.EXE", editor);
        Assert.Equal(["outlook.exe"], editor.Apps);
        modes.Save();
        var save = Assert.Single(ModesJson.Saves(sent));
        var fields = ModesJson.ModeOf(save);
        Assert.False(fields.ContainsKey("id"));
        Assert.Equal("Email", fields["name"]!.GetValue<string>());
        Assert.Equal("relaxed", fields["style"]!.GetValue<string>());
        Assert.True(fields["polish"]!.GetValue<bool>());
        Assert.True(fields["remove_fillers"]!.GetValue<bool>());
        Assert.Equal("Keep it short.\rNo emoji.", fields["polish_prompt"]!.GetValue<string>());
        Assert.Equal(["outlook.exe"], fields["apps"]!.AsArray().Select(a => a!.GetValue<string>()));
        Assert.True(fields.ContainsKey("polish_model"));
        Assert.Null(fields["polish_model"]); // the AI setting's
        Assert.Null(fields["polish_model_name"]);
        Assert.False(fields.ContainsKey("polish_model_confirm"));
        Assert.False(save.ContainsKey("take_apps"));
        Assert.True(editor.Saving);
        modes.Save();
        Assert.Single(ModesJson.Saves(sent)); // one save at a time

        modes.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat"), ModesJson.Default], reference: "modes:99"));
        Assert.NotNull(modes.Editor); // not the save's answer
        modes.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat"), ModesJson.Mode("m1", "Email"), ModesJson.Default], reference: "modes:1", saved: "m1"));
        Assert.Null(modes.Editor);
        Assert.Equal(["Chat", "Email", "Default"], modes.Rows.Select(r => r.Name));
    }

    [Fact]
    public void ChangingAModeNamesOnlyWhatChanged()
    {
        var (modes, sent, _) = Model([ModesJson.Mode("c", "Chat", style: "other", prompt: "Mine.", apps: ["outlook.exe"]), ModesJson.Default]);
        modes.Edit("c");
        var editor = modes.Editor!;
        Assert.Equal(ModeStyle.Other, editor.Style);
        Assert.False(editor.Renamed);
        editor.Name = "Chat 2";
        Assert.True(editor.Renamed);
        modes.Save();
        Assert.Equal(["id", "name"], ModesJson.ModeOf(ModesJson.Saves(sent)[^1]).Select(f => f.Key).Order());

        modes.CloseEditor();
        modes.Edit("d");
        var fallback = modes.Editor!;
        Assert.True(fallback.IsDefault);
        fallback.Name = "Everything";
        ModesModel.AddApp("outlook.exe", fallback);
        Assert.Empty(fallback.Apps); // every other app's
        modes.Save();
        Assert.Equal(["id", "name"], ModesJson.ModeOf(ModesJson.Saves(sent)[^1]).Select(f => f.Key).Order());
    }

    [Fact]
    public void PickingAnotherModesAppMovesIt()
    {
        var slack = new PickedApp("slack.exe", "Slack", null);
        var notes = new PickedApp("onenote.exe", "OneNote", null);
        var (modes, sent, _) = Model(running: [slack, notes]);
        modes.Add();
        var editor = modes.Editor!;
        editor.Name = "Work";
        var offers = modes.RunningOffers(editor);
        Assert.Equal(["Slack", "OneNote"], offers.Select(o => o.App.Name));
        Assert.Equal(["Chat", null], offers.Select(o => o.Owner));
        ModesModel.AddApp(slack.Identity, editor);
        Assert.Equal(["OneNote"], modes.RunningOffers(editor).Select(o => o.App.Name));
        Assert.Equal(["Moves Slack from Chat."], modes.MovingNotes(editor));
        modes.Save();
        Assert.True(ModesJson.Saves(sent)[^1]["take_apps"]!.GetValue<bool>());

        modes.Apply(ModesJson.Failed("modes.save", "modes:1", "name_taken"));
        ModesModel.RemoveApp(slack.Identity, editor);
        Assert.Empty(modes.MovingNotes(editor));
        modes.Save();
        Assert.False(ModesJson.Saves(sent)[^1].ContainsKey("take_apps"));
    }

    [Fact]
    public void AnAppTakenMeanwhileIsMovedOnTheNextSaveOnly()
    {
        var (modes, sent, _) = Model();
        modes.Add();
        var editor = modes.Editor!;
        editor.Name = "Work";
        ModesModel.AddApp("onenote.exe", editor);
        modes.Save();
        modes.Apply(ModesJson.Failed("modes.save", "modes:1", "app_taken"));
        Assert.Equal("An app here is in another mode now. Save again to move it here.", editor.Error);
        Assert.False(editor.Saving);
        Assert.Equal(new CoreCommand.ModesList("modes:2"), sent.Commands[^1]); // which mode has it now
        modes.Save();
        Assert.True(ModesJson.Saves(sent)[^1]["take_apps"]!.GetValue<bool>());
        modes.Apply(ModesJson.Failed("modes.save", "modes:3", "name_taken"));
        modes.Save();
        Assert.False(ModesJson.Saves(sent)[^1].ContainsKey("take_apps")); // said once, used once
    }

    /// <summary>Verify: every refusal the core can give a save, a delete or a confirm is said in words of its own, never the core's message or its code.</summary>
    [Fact]
    public void EveryFailureCodeHasItsOwnWords()
    {
        var editor = new ModeEditor(null, isDefault: false);
        var expected = new Dictionary<FailureCode, string>
        {
            [FailureCode.NameBlank] = "Give the mode a name.",
            [FailureCode.NameTaken] = "Another mode has a name that sounds the same. Pick another name.",
            [FailureCode.NameIsStyle] = "Formal, Casual and Relaxed name the styles in voice commands. Pick another name.",
            [FailureCode.TooLong] = "You can have up to 50 modes.",
            [FailureCode.DefaultMode] = "Everywhere else is used in every app without a mode of its own, so it can't be given apps.",
            [FailureCode.AppTaken] = "An app here is in another mode now. Save again to move it here.",
            [FailureCode.AppInvalid] = "One of these apps can't be told apart from others. Remove it, and pick it again.",
            [FailureCode.ModeNotFound] = "This mode was deleted meanwhile, so it wasn't saved.",
            [FailureCode.ModelUnknown] = "That model isn't available any more. Pick another.",
            [FailureCode.ModelNameInvalid] = "A model name can be up to 128 characters, on one line.",
            [FailureCode.DestinationChanged] = "Where its model sends changed while you looked. Check it, and confirm again.",
            [FailureCode.ListUnreadable] = "Your modes can't be read, so this wasn't saved. Start over replaces them with the default.",
            [FailureCode.MeetingRecording] = "Couldn't save the mode. Try again.",
            [FailureCode.DeleteWindowOver] = "Couldn't save the mode. Try again.",
        };
        foreach (var code in Enum.GetValues<FailureCode>())
        {
            var words = ModesModel.SaveFailure(code, editor);
            Assert.Equal(expected[code], words);
            Assert.DoesNotContain("_", words, StringComparison.Ordinal);
        }
        Assert.Equal("Couldn't save the mode. Try again.", ModesModel.SaveFailure(null, editor));
        editor.Name = new string('n', 65);
        Assert.Equal("A name can be up to 64 characters.", ModesModel.SaveFailure(FailureCode.TooLong, editor));
        editor.Name = "Fine";
        editor.Prompt = new string('p', 2001);
        Assert.Equal("Polish instructions can be up to 2,000 characters.", ModesModel.SaveFailure(FailureCode.TooLong, editor));
        editor.Prompt = "";
        editor.SetApps(Enumerable.Range(0, 65).Select(i => $"app{i}.exe"));
        Assert.Equal("Shorten to 64 apps or fewer.", ModesModel.SaveFailure(FailureCode.TooLong, editor));
        var existing = new ModeEditor(JsonSerializer.Deserialize(ModesJson.Mode("c", "Chat"), InkEventsJson.Default.ModeInfo), isDefault: false);
        Assert.Equal("Something here is too long. Shorten it.", ModesModel.SaveFailure(FailureCode.TooLong, existing));
        // Counted as the core counts: a flag is two scalars (four UTF-16 units).
        existing.Prompt = string.Concat(Enumerable.Repeat("\U0001F1E9\U0001F1F0", 1001));
        Assert.Equal("Polish instructions can be up to 2,000 characters.", ModesModel.SaveFailure(FailureCode.TooLong, existing));
        existing.Prompt = string.Concat(Enumerable.Repeat("\U0001F600", 1500));
        Assert.False(existing.PromptTooLong); // 3,000 UTF-16 units, 1,500 scalars

        Assert.Equal("Everywhere else can't be deleted: it is used in every app without a mode of its own.", ModesModel.DeleteFailure(FailureCode.DefaultMode, "Default"));
        Assert.Equal("“Chat” was already deleted.", ModesModel.DeleteFailure(FailureCode.ModeNotFound, "Chat"));
        Assert.Equal("Your modes can't be read, so “Chat” wasn't deleted. Start over replaces them with the default.", ModesModel.DeleteFailure(FailureCode.ListUnreadable, "Chat"));
        Assert.Equal("Couldn't delete “Chat”. Try again.", ModesModel.DeleteFailure(null, "Chat"));
        Assert.Equal("Where its model sends changed while you looked. Check it, and confirm again.", ModesModel.ConfirmFailure(FailureCode.DestinationChanged));
        Assert.Equal("That mode was deleted meanwhile.", ModesModel.ConfirmFailure(FailureCode.ModeNotFound));
        Assert.Equal("Its model isn't available any more. Edit the mode to pick another.", ModesModel.ConfirmFailure(FailureCode.ModelUnknown));
        Assert.Equal("Your modes can't be read, so nothing was confirmed. Start over replaces them with the default.", ModesModel.ConfirmFailure(FailureCode.ListUnreadable));
        Assert.Equal("Couldn't confirm its model. Try again.", ModesModel.ConfirmFailure(null));
    }

    [Fact]
    public void ARefusalShowsInTheEditorByItsCode()
    {
        var (modes, sent, _) = Model();
        modes.Edit("c");
        var editor = modes.Editor!;
        editor.Name = "casual";
        modes.Save();
        modes.Apply(ModesJson.Failed("modes.save", "modes:other", "name_blank"));
        Assert.Null(editor.Error);
        Assert.True(editor.Saving);
        modes.Apply(ModesJson.Failed("modes.save", "modes:1", "name_is_style"));
        Assert.Equal(ModesModel.SaveFailure(FailureCode.NameIsStyle, editor), editor.Error);
        Assert.DoesNotContain("zebra", editor.Error, StringComparison.Ordinal);
        Assert.False(editor.Saving);
        foreach (var code in new[] { "mode_not_found", "model_unknown", "destination_changed" })
        {
            sent.Commands.Clear();
            modes.Save();
            var reference = sent.Commands[0].CommandId!;
            modes.Apply(ModesJson.Failed("modes.save", reference, code));
            Assert.Contains(sent.Commands, c => c is CoreCommand.ModesList); // read again
        }
        // Cancelled while it was on its way: the refusal is said in the section.
        sent.Commands.Clear();
        modes.Save();
        var late = sent.Commands[0].CommandId!;
        modes.CloseEditor();
        modes.Apply(ModesJson.Failed("modes.save", late, "name_blank"));
        Assert.Null(modes.Editor);
        Assert.Equal("Your change to “Chat” wasn't saved. Give the mode a name.", modes.Problem);
    }

    /// <summary>A model at a destination no polish consent covers asks first; Allow records the OK, and the save follows once the core lists it; Cancel records and saves nothing.</summary>
    [Fact]
    public void AModelAtANewDestinationAsksForItsOwnOkBeforeTheSave()
    {
        var (modes, sent, consent) = Model(models: [ModesJson.Apple, ModesJson.Groq(false)]);
        modes.Edit("c");
        var editor = modes.Editor!;
        Assert.Equal(
            ["As in AI (Example Local Model, on this PC)", "Example Local Model, on this PC", "Groq · llama-3.1-8b-instant"],
            modes.ModelOptions(editor).Select(o => o.Label));
        editor.PolishModel = "provider:groq";
        Assert.Equal("llama-3.1-8b-instant", modes.ProviderModel(editor));
        Assert.Equal("Saving asks for your OK to send this mode's words to Groq.", modes.ModelNote(editor).Text);
        modes.Save();
        Assert.Equal(ConsentDestination.Cloud("https://api.groq.com/openai/v1", "Groq"), editor.ConsentStep);
        Assert.Equal("Polish “Chat” with Groq · llama-3.1-8b-instant?", modes.ConsentTitle(editor));
        Assert.Empty(sent.Commands);
        modes.CancelConsentStep();
        Assert.Null(editor.ConsentStep);
        Assert.Empty(sent.Commands);

        modes.Save();
        modes.AllowAndSave(editor.ConsentStep!);
        Assert.Equal([new CoreCommand.ConsentAllow(LlmFeature.Polish, LlmDestination.Cloud, "https://api.groq.com/openai/v1", null, "consent.allow:polish:1")], sent.Commands);
        Assert.True(editor.Saving);
        var answer = ModesJson.PolishState(true, [ModesJson.DeviceConsent, ModesJson.GroqConsent], "consent.allow:polish:1");
        consent.Apply(answer);
        modes.Apply(answer);
        Assert.Equal(2, consent.State?.Consents.Count);
        var fields = ModesJson.ModeOf(ModesJson.Saves(sent)[^1]);
        Assert.Equal("provider:groq", fields["polish_model"]!.GetValue<string>());
        Assert.Null(fields["polish_model_name"]);

        editor.PolishModelName = " llama-3.3-70b ";
        Assert.Equal("llama-3.3-70b", editor.ModelNameToSend);
        editor.PolishModel = "engine:local-llm";
        Assert.Null(editor.ModelNameToSend); // only a provider's
    }

    [Fact]
    public void AnOkTheCoreDidNotRecordSavesNothing()
    {
        var (modes, sent, _) = Model(models: [ModesJson.Apple, ModesJson.Groq(false)]);
        modes.Edit("c");
        var editor = modes.Editor!;
        editor.PolishModel = "provider:groq";
        modes.Save();
        modes.AllowAndSave(editor.ConsentStep!);
        modes.Apply(ModesJson.PolishState(true, [ModesJson.DeviceConsent], "consent.allow:polish:1"));
        Assert.Equal(ModesModel.OkFailure, editor.Error);
        Assert.False(editor.Saving);
        Assert.Empty(ModesJson.Saves(sent));
        modes.Save();
        modes.AllowAndSave(editor.ConsentStep!);
        modes.Apply(ModesJson.Failed("consent.allow", "consent.allow:polish:2", null));
        Assert.Equal("Couldn't record your OK, so nothing was saved. Try again.", editor.Error);
        Assert.Empty(ModesJson.Saves(sent));
    }

    [Fact]
    public void AnUnchangedModelDoesNotAskAgain()
    {
        var (modes, sent, _) = Model([ModesJson.Mode("c", "Chat", model: "provider:groq", state: "ready"), ModesJson.Default], [ModesJson.Apple, ModesJson.Groq(false)]);
        modes.Edit("c");
        var editor = modes.Editor!;
        editor.Name = "Chats";
        Assert.Equal(("Polish needs your OK to send this mode's words to Groq.", true), modes.ModelNote(editor));
        modes.Save();
        Assert.Null(editor.ConsentStep);
        Assert.Equal(["id", "name"], ModesJson.ModeOf(Assert.Single(ModesJson.Saves(sent))).Select(f => f.Key).Order());
    }

    [Fact]
    public void TheEditorSaysWhenNothingWouldPolish()
    {
        var (none, _, _) = Model(models: [], setting: null);
        none.Edit("c");
        Assert.Equal("No language model is available on this PC, so nothing is polished.", none.PolishNote(none.Editor!));
        Assert.Equal(["As in AI (none now)"], none.ModelOptions(none.Editor!).Select(o => o.Label));
        var (off, _, _) = Model(switchOn: false);
        off.Edit("c");
        var editor = off.Editor!;
        Assert.Equal("Polish my words is off in AI, so nothing is polished.", off.PolishNote(editor));
        editor.Polish = false;
        Assert.Null(off.PolishNote(editor));
        Assert.Equal("Uses the model chosen in AI. Your words stay on this PC.", off.ModelNote(editor).Text);
    }

    [Fact]
    public void AMissingModelStaysInThePicker()
    {
        var (modes, _, _) = Model([ModesJson.Mode("c", "Chat", model: "provider:openai", state: "missing"), ModesJson.Default]);
        modes.Edit("c");
        var editor = modes.Editor!;
        Assert.Equal(("provider:openai", "OpenAI (not available)"), modes.ModelOptions(editor)[^1]);
        Assert.Equal(("This model isn't available now, so this mode isn't polished. Pick another.", true), modes.ModelNote(editor));
        Assert.False(modes.CanConfirmInEditor(editor));
    }

    [Fact]
    public void ConfirmingAMovedModelFromTheRow()
    {
        var (modes, sent, _) = Model([ModesJson.Mode("c", "Chat", model: "provider:groq", state: "moved"), ModesJson.Default], [ModesJson.Apple, ModesJson.Groq(true)]);
        modes.AskConfirm("c");
        var c = modes.Confirming!;
        Assert.True(c.Pin);
        Assert.False(c.AsksOk);
        Assert.Equal("Polish “Chat” with Groq · llama-3.1-8b-instant?", ModesModel.ConfirmTitle(c));
        Assert.Equal("Send to Groq", ModesModel.ConfirmButton(c));
        Assert.Equal(ConsentModel.Message(LlmFeature.Polish, c.Choice.Destination), modes.ConfirmMessage(c));
        modes.ConfirmAllow(c);
        Assert.Null(modes.Confirming);
        var fields = ModesJson.ModeOf(ModesJson.Saves(sent)[^1]);
        Assert.Equal(["id", "polish_model_confirm", "polish_model_confirm_to"], fields.Select(f => f.Key).Order());
        Assert.True(fields["polish_model_confirm"]!.GetValue<bool>());
        Assert.Equal("cloud", fields["polish_model_confirm_to"]!["to"]!.GetValue<string>());
        Assert.Equal("https://api.groq.com/openai/v1", fields["polish_model_confirm_to"]!["endpoint"]!.GetValue<string>());
        modes.Apply(ModesJson.Failed("modes.save", "modes:1", "destination_changed"));
        Assert.Equal(ModesModel.ConfirmFailure(FailureCode.DestinationChanged), modes.Problem);
        Assert.Equal(new CoreCommand.ModesList("modes:2"), sent.Commands[^1]);

        var (asking, asked, _) = Model([ModesJson.Mode("c", "Chat", model: "provider:groq", state: "unrecorded"), ModesJson.Default], [ModesJson.Apple, ModesJson.Groq(false)], switchOn: false);
        asking.AskConfirm("c");
        var both = asking.Confirming!;
        Assert.True(both.AsksOk);
        Assert.EndsWith(" This also turns on Polish my words.", asking.ConfirmMessage(both), StringComparison.Ordinal);
        asking.ConfirmAllow(both);
        Assert.Equal([new CoreCommand.ConsentAllow(LlmFeature.Polish, LlmDestination.Cloud, "https://api.groq.com/openai/v1", null, "consent.allow:polish:1")], asked.Commands);
        asking.Apply(ModesJson.PolishState(true, [ModesJson.GroqConsent], "consent.allow:polish:1"));
        Assert.True(ModesJson.ModeOf(ModesJson.Saves(asked)[^1])["polish_model_confirm"]!.GetValue<bool>());
    }

    [Fact]
    public void AllowingFromTheRowRecordsOnlyTheOk()
    {
        var (modes, sent, _) = Model([ModesJson.Mode("c", "Chat", model: "provider:groq", state: "ready"), ModesJson.Default], [ModesJson.Apple, ModesJson.Groq(false)]);
        modes.AskConfirm("c");
        Assert.False(modes.Confirming!.Pin);
        modes.ConfirmAllow(modes.Confirming!);
        modes.Apply(ModesJson.PolishState(true, [ModesJson.GroqConsent], "consent.allow:polish:1"));
        Assert.Empty(ModesJson.Saves(sent));
        Assert.Null(modes.Problem);
        Assert.False(modes.Busy);
        modes.AskConfirm("c");
        modes.ConfirmAllow(modes.Confirming!);
        modes.Apply(ModesJson.PolishState(true, [], "consent.allow:polish:2"));
        Assert.Equal(ModesModel.OkFailureRow, modes.Problem);
    }

    [Fact]
    public void AnEditorConfirmSendsWhatWasShownAndAsksAgainWhenItMoved()
    {
        var (modes, sent, _) = Model([ModesJson.Mode("c", "Chat", model: "provider:groq", state: "moved"), ModesJson.Default], [ModesJson.Apple, ModesJson.Groq(true)]);
        modes.Edit("c");
        var editor = modes.Editor!;
        Assert.True(modes.CanConfirmInEditor(editor));
        modes.ConfirmInEditor(editor);
        Assert.True(modes.Confirmed(editor));
        const string Moved = """{"id":"provider:groq","name":"Groq","model":"llama-3.1-8b-instant","to":"cloud","endpoint":"https://elsewhere.example.com/v1","allowed":true,"blocked_local_only":false}""";
        modes.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat", model: "provider:groq", state: "moved"), ModesJson.Default], [ModesJson.Apple, Moved]));
        Assert.False(modes.Confirmed(editor)); // a listing overtook it: Confirm asks again
        Assert.True(modes.CanConfirmInEditor(editor));
        modes.Save();
        Assert.Equal("https://api.groq.com/openai/v1", ModesJson.ModeOf(ModesJson.Saves(sent)[^1])["polish_model_confirm_to"]!["endpoint"]!.GetValue<string>());
        modes.Apply(ModesJson.Failed("modes.save", "modes:1", "destination_changed"));
        Assert.Null(editor.ConfirmedTo);
        modes.Save();
        Assert.False(ModesJson.ModeOf(ModesJson.Saves(sent)[^1]).ContainsKey("polish_model_confirm"));
    }

    [Fact]
    public void ConfirmingInTheEditor()
    {
        var (modes, sent, _) = Model([ModesJson.Mode("c", "Chat", model: "engine:local-llm", state: "unrecorded"), ModesJson.Default]);
        modes.Edit("c");
        var editor = modes.Editor!;
        Assert.True(modes.CanConfirmInEditor(editor));
        Assert.True(modes.ModelNote(editor).IsProblem);
        modes.ConfirmInEditor(editor);
        Assert.True(editor.Confirmed);
        Assert.False(modes.CanConfirmInEditor(editor));
        modes.Save();
        var fields = ModesJson.ModeOf(ModesJson.Saves(sent)[^1]);
        Assert.Equal("on_device", fields["polish_model_confirm_to"]!["to"]!.GetValue<string>());
        Assert.False(fields["polish_model_confirm_to"]!.AsObject().ContainsKey("endpoint"));
        Assert.False(fields.ContainsKey("polish_model")); // the same pin, confirmed
    }

    [Fact]
    public void RowOperationsWaitForTheirAnswer()
    {
        var (modes, sent, _) = Model([ModesJson.Mode("c", "Chat"), ModesJson.Mode("n", "Notes"), ModesJson.Default]);
        modes.AskDelete("c");
        modes.Delete(modes.Deleting!);
        Assert.True(modes.Busy);
        modes.AskDelete("n");
        Assert.Null(modes.Deleting);
        modes.Apply(ModesJson.Failed("modes.delete", "modes:1", null));
        Assert.False(modes.Busy);
        Assert.Equal("Couldn't delete “Chat”. Try again.", modes.Problem);
        modes.AskDelete("n");
        modes.Delete(modes.Deleting!);
        modes.Apply(ModesJson.Listing([ModesJson.Mode("c", "Chat"), ModesJson.Default], reference: "modes:2"));
        Assert.False(modes.Busy);
        Assert.Null(modes.Problem);
        Assert.Equal(2, sent.Commands.OfType<CoreCommand.ModesDelete>().Count());
    }

    [Fact]
    public void OverlappingSavesKeepTheirOwnRefusalsAndAStoppedCoreLeavesNothingWaiting()
    {
        var (modes, _, _) = Model();
        modes.Edit("c");
        var first = modes.Editor!;
        first.Name = "Casual";
        modes.Save();
        modes.CloseEditor();
        modes.Add();
        var second = modes.Editor!;
        modes.Save();
        modes.Apply(ModesJson.Failed("modes.save", "modes:1", "name_is_style"));
        Assert.Equal($"Your change to “Chat” wasn't saved. {ModesModel.SaveFailure(FailureCode.NameIsStyle, first)}", modes.Problem);
        Assert.True(second.Saving);
        modes.Apply(Ev.Of("""{"type":"core.stopped"}"""));
        Assert.False(second.Saving);
        Assert.False(modes.Busy);
    }

    [Fact]
    public void TheEditorsCountPlaceholderAndBrowsedApps()
    {
        var (modes, _, _) = Model();
        modes.Add();
        var editor = modes.Editor!;
        Assert.Equal("Tidy it.", modes.DefaultPrompt);
        editor.Prompt = new string('a', 120);
        Assert.Equal($"{120:N0} / {2000:N0}", editor.PromptCount);
        Assert.False(editor.PromptTooLong);
        ModesModel.AddBrowsed(@"C:\Program Files\Example\ExampleWriter.exe", editor);
        Assert.Equal(["examplewriter.exe"], editor.Apps);
        ModesModel.AddBrowsed(@"C:\Program Files\Inkwell\Inkwell.exe", editor);
        Assert.Equal("Inkwell itself needs no mode.", editor.Error);
        ModesModel.AddBrowsed(@"C:\Docs\notes.txt", editor);
        Assert.Equal("That isn't an app Inkwell can tell is in front. Pick its .exe.", editor.Error);
        Assert.Equal(["examplewriter.exe"], editor.Apps);
    }
}

/// <summary>The editor's commands against the real core: the fields it sends are the ones the core reads, and each refusal comes back with its code. A fresh library, with no language model.</summary>
public class ModesCoreContractTests
{
    [Fact]
    public void TheEditorAgainstTheRealCore()
    {
        var data = Path.Combine(Path.GetTempPath(), $"inkwell-modes-{Guid.NewGuid():N}");
        var events = new Events();
        var session = PhrasesCoreContractTests.Start(new InkConfig(data, LogLevel: "warn"), events.Record);
        try
        {
            var applied = 0;
            var modes = new ModesModel(c => session.Command(c.Json), polishSwitch: () => true);
            void Pump(Func<bool> done)
            {
                var until = DateTime.UtcNow + TimeSpan.FromSeconds(10);
                while (DateTime.UtcNow < until)
                {
                    var all = events.All;
                    for (; applied < all.Count; applied++)
                    {
                        modes.Apply(all[applied]);
                    }
                    if (done())
                    {
                        return;
                    }
                    Thread.Sleep(10);
                }
                Assert.Fail("the core never answered");
            }
            string? Refused()
            {
                modes.Save();
                Pump(() => modes.Editor?.Saving == false);
                return modes.Editor?.Error;
            }

            modes.Load();
            Pump(() => modes.Rows.Count > 0);
            Assert.Equal(["Default"], modes.Rows.Select(r => r.Name));
            Assert.NotEmpty(modes.DefaultPrompt);
            Assert.Null(modes.Rows[0].Polish.Chip); // no language model here

            modes.Add();
            var editor = modes.Editor!;
            editor.Name = "Chat";
            editor.Style = ModeStyle.Casual;
            editor.Polish = true;
            editor.Prompt = "Short.\rNo emoji.";
            ModesModel.AddApp("examplechat.exe", editor);
            modes.Save();
            Pump(() => modes.Editor is null);
            Assert.Equal(["Chat", "Default"], modes.Rows.Select(r => r.Name));
            Assert.Equal(["examplechat.exe"], modes.Rows[0].Apps.Select(a => a.Id));
            Assert.Equal("Everywhere else", modes.Rows[1].Title);
            var chat = modes.Rows[0].Id;

            modes.Edit(chat);
            editor = modes.Editor!;
            editor.Name = "  ";
            Assert.Equal(ModesModel.SaveFailure(FailureCode.NameBlank, editor), Refused());
            editor.Name = "Casual";
            Assert.Equal(ModesModel.SaveFailure(FailureCode.NameIsStyle, editor), Refused());
            editor.Name = "default ";
            Assert.Equal(ModesModel.SaveFailure(FailureCode.NameTaken, editor), Refused());
            editor.Name = new string('n', 65);
            Assert.Equal("A name can be up to 64 characters.", Refused());
            editor.Name = "Chat";
            editor.Prompt = new string('p', 2001);
            Assert.Equal("Polish instructions can be up to 2,000 characters.", Refused());
            editor.Prompt = "Short.";
            editor.SetApps(["x"]);
            Assert.Equal(ModesModel.SaveFailure(FailureCode.AppInvalid, editor), Refused());
            editor.SetApps(["examplechat.exe"]);
            editor.PolishModel = "provider:groq";
            Assert.Equal(ModesModel.SaveFailure(FailureCode.ModelUnknown, editor), Refused());
            editor.PolishModel = null;
            modes.Save();
            Pump(() => modes.Editor is null);

            modes.Edit("default");
            modes.Editor!.Name = "Everything";
            modes.Save();
            Pump(() => modes.Editor is null);
            Assert.Equal("Everything", modes.Rows[^1].Name);
            Assert.Equal("Everywhere else", modes.Rows[^1].Title);

            modes.Add();
            editor = modes.Editor!;
            editor.Name = "Work";
            ModesModel.AddApp("examplechat.exe", editor);
            Assert.Equal(["Moves Examplechat from Chat."], modes.MovingNotes(editor));
            modes.Save();
            Pump(() => modes.Editor is null);
            Assert.Empty(modes.Rows.First(r => r.Name == "Chat").Apps);
            Assert.Equal(["examplechat.exe"], modes.Rows.First(r => r.Name == "Work").Apps.Select(a => a.Id));

            var work = modes.Rows.First(r => r.Name == "Work").Id;
            modes.Edit(work);
            editor = modes.Editor!;
            modes.AskDelete(work);
            modes.Delete(modes.Deleting!);
            Pump(() => modes.Rows.All(r => r.Id != work));
            editor.Name = "Work again";
            Assert.Equal(ModesModel.SaveFailure(FailureCode.ModeNotFound, editor), Refused());
            modes.CloseEditor();

            // Polish's consents: a revoke of one not there is no failure.
            var consent = new ConsentModel(LlmFeature.Polish, PolishModel.SettingId, c => session.Command(c.Json));
            var before = events.All.Count;
            consent.Revoke(new ConsentGrant(LlmDestination.OnDevice, null, null));
            var until = DateTime.UtcNow + TimeSpan.FromSeconds(10);
            ConsentState? answered = null;
            while (answered is null && DateTime.UtcNow < until)
            {
                answered = events.All.Skip(before).OfType<ConsentState>().FirstOrDefault(s => s.Ref == "consent.revoke:polish:1");
                Thread.Sleep(10);
            }
            Assert.NotNull(answered);
            consent.Apply(answered);
            Assert.Null(consent.Failure);
            Assert.Empty(consent.State!.Consents);
            Assert.DoesNotContain(events.All, e => e is UndecodableEvent or UnknownEvent);
        }
        finally
        {
            session.Shutdown();
            if (Directory.Exists(data))
            {
                Directory.Delete(data, recursive: true);
            }
        }
    }
}
