// Inkwell 0.2's import on Windows (the Mac's Import02Tests): what the first run's step and Settings
// > Voice show for the core's answers, and what they send. Synthetic events only: no test here
// starts a core, since a real core looks in this PC's own 0.2 data folder.
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class Import02ModelTests
{
    private const string Counts =
        """{"dictations":12,"dictionary_entries":0,"snippets":1,"modes":2,"settings":4,"voice_commands":0,"app_style_rules":0,"linked_keys":0}""";

    internal static InkEvent Checked(string state, string? counts = null, string? message = null, string reference = "import.check")
    {
        var fields = new List<string> { "\"type\":\"import.checked\"", $"\"state\":\"{state}\"", $"\"ref\":\"{reference}\"" };
        if (counts is not null)
        {
            fields.Add($"\"counts\":{counts}");
        }
        if (message is not null)
        {
            fields.Add($"\"message\":\"{message}\"");
        }
        return Ev.Of("{" + string.Join(",", fields) + "}");
    }

    internal static InkEvent Found => Checked("found", Counts);

    internal static InkEvent Finished(string reference = "import.run") => Ev.Of(
        $$"""{"type":"import.finished","counts":{"dictations":12,"dictionary_entries":0,"snippets":1,"modes":2,"settings":4,"voice_commands":0,"app_style_rules":0,"linked_keys":1},"ref":"{{reference}}"}""");

    [Fact]
    public void FoundDataIsOfferedWithItsCountsInWords()
    {
        var sent = new Sent();
        var model = new Import02Model(sent.Send);
        Assert.False(model.Offered); // nothing until the core answers
        model.Check();
        Assert.Equal([new CoreCommand.ImportCheck()], sent.Commands);
        model.Apply(Found);
        Assert.True(model.Offered);
        Assert.True(model.CanImport);
        Assert.Equal(
            "Inkwell 0.2 left 12 dictations, 1 snippet, 2 modes and your settings on this PC. Import brings them into this library; 0.2’s own copy stays as it is.",
            model.Line);
    }

    [Theory]
    [InlineData("absent")]
    [InlineData("imported")]
    public void NothingToImportIsNotOffered(string state)
    {
        var model = new Import02Model(_ => { });
        model.Apply(Checked(state));
        Assert.False(model.Offered);
        Assert.False(model.ShownInSettings);
        Assert.False(model.CanImport);
        Assert.Equal("", model.Line);
    }

    [Fact]
    public void DataThatCannotBeReadNowSaysWhy()
    {
        var model = new Import02Model(_ => { });
        model.Apply(Checked("unreadable", message: "Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again"));
        Assert.True(model.Offered); // Import may work once the reason is gone
        Assert.True(model.CanImport);
        Assert.Equal(
            "Inkwell 0.2’s data is on this PC but can’t be read now: Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again.",
            model.Line);
    }

    [Fact]
    public void AnImportSaysWhatCameOverAndRunsOnce()
    {
        var sent = new Sent();
        var model = new Import02Model(sent.Send);
        model.Apply(Found);
        model.Run();
        Assert.Equal([new CoreCommand.ImportRun()], sent.Commands);
        Assert.False(model.CanImport); // not twice at once
        model.Run();
        model.Check();
        Assert.Equal([new CoreCommand.ImportRun()], sent.Commands); // no second import, and no look while one runs
        model.Apply(Finished());
        Assert.True(model.Offered); // what came over is said where it was pressed
        Assert.False(model.CanImport);
        Assert.Equal("Brought over from Inkwell 0.2: 12 dictations, 1 snippet, 2 modes, your settings and 1 saved API key.", model.Line);
        // A look after it says imported; the report stays for this session.
        model.Check();
        model.Apply(Checked("imported"));
        Assert.True(model.Offered);
        model.Run();
        Assert.Equal([new CoreCommand.ImportRun(), new CoreCommand.ImportCheck()], sent.Commands);
    }

    [Fact]
    public void AFailedImportIsSaidInWordsAndCanBeTriedAgain()
    {
        var sent = new Sent();
        var model = new Import02Model(sent.Send);
        model.Apply(Found);
        model.Run();
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"import.run","id":"import.run","message":"Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again"}""");
        Assert.True(Import02Model.Handles(failed));
        model.Apply(failed);
        Assert.Equal("Couldn’t import: Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again.", model.Failure);
        Assert.True(model.CanImport);
        model.Run();
        Assert.Equal([new CoreCommand.ImportRun(), new CoreCommand.ImportRun()], sent.Commands);
        Assert.Null(model.Failure); // cleared while it tries again
    }

    [Fact]
    public void ALaterLookThatCanReadTheDataClearsAnOldFailure()
    {
        var sent = new Sent();
        var model = new Import02Model(sent.Send);
        model.Apply(Found);
        model.Run();
        model.Apply(Ev.Of("""{"type":"command.failed","command":"import.run","id":"import.run","message":"Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again"}"""));
        Assert.NotNull(model.Failure);
        // 0.2 quit, Settings opened again: the look reads the data now.
        model.Check();
        Assert.Equal([new CoreCommand.ImportRun(), new CoreCommand.ImportCheck()], sent.Commands);
        model.Apply(Found);
        Assert.Null(model.Failure); // the old failure no longer applies
        Assert.True(model.CanImport);
        Assert.Equal(
            "Inkwell 0.2 left 12 dictations, 1 snippet, 2 modes and your settings on this PC. Import brings them into this library; 0.2’s own copy stays as it is.",
            model.Line);
    }

    [Fact]
    public void ALookThatFailedIsLoggedAndShownOnlyInSettings()
    {
        var logged = new Logged();
        var model = new Import02Model(_ => { }, logged.Log);
        model.Check();
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"import.check","id":"import.check","message":"the library could not be read"}""");
        Assert.True(Import02Model.Handles(failed));
        model.Apply(failed);
        Assert.True(model.CheckFailed);
        Assert.False(model.Offered); // the first run goes without the step
        Assert.True(model.ShownInSettings);
        Assert.True(model.LineIsProblem);
        Assert.Equal("Couldn’t look for Inkwell 0.2’s data.", model.Line);
        var line = Assert.Single(logged.Messages);
        Assert.Contains("import.check", line, StringComparison.Ordinal);
    }

    [Fact]
    public void AnswersToOtherCommandsAreNotTheImports()
    {
        var model = new Import02Model(_ => { });
        model.Apply(Checked("found", Counts, reference: "someone-else"));
        model.Apply(Finished("someone-else"));
        model.Apply(Ev.Of("""{"type":"command.failed","command":"import.notes","id":"import.notes","message":"x"}"""));
        Assert.Null(model.Found);
        Assert.Null(model.Imported);
        Assert.False(model.CheckFailed);
        Assert.Null(model.Failure);
        Assert.False(Import02Model.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"import.notes","message":"x"}""")));
    }

    [Fact]
    public void TheFirstRunLooksOnceAndAMovedLibraryNever()
    {
        var sent = new Sent();
        var model = new Import02Model(sent.Send);
        model.CheckOnce();
        model.CheckOnce();
        Assert.Equal([new CoreCommand.ImportCheck()], sent.Commands);

        var moved = new Sent();
        var never = new Import02Model(moved.Send) { Looks = false };
        never.CheckOnce();
        never.Check();
        never.Run();
        Assert.Empty(moved.Commands);
        Assert.False(never.Offered);
    }

    [Fact]
    public void TheCommandsCarryTheirNameAsTheirIdAndNoPath()
    {
        foreach (var (command, name) in new (CoreCommand, string)[] { (new CoreCommand.ImportCheck(), "import.check"), (new CoreCommand.ImportRun(), "import.run") })
        {
            var fields = JsonDocument.Parse(command.Json).RootElement;
            Assert.Equal(["cmd", "id"], fields.EnumerateObject().Select(p => p.Name).Order(StringComparer.Ordinal));
            Assert.Equal(name, fields.GetProperty("cmd").GetString());
            Assert.Equal(name, fields.GetProperty("id").GetString());
            Assert.Equal(name, command.Name);
            Assert.Equal(name, command.CommandId);
        }
    }

    [Fact]
    public void TheFirstRunShowsTheStepOnlyWhileThereIsSomethingToImport()
    {
        var screens = new ScreenModels(_ => { }, log: new Logged().Log);
        var onboarding = screens.Onboarding;
        Assert.Equal([OnboardingStep.Welcome, OnboardingStep.Permissions, OnboardingStep.Models, OnboardingStep.Polish, OnboardingStep.Ready], onboarding.ShownSteps);
        onboarding.Next();
        onboarding.Next();
        onboarding.Next();
        Assert.Equal(OnboardingStep.Polish, onboarding.Step); // no 0.2 data: no step

        screens.Apply([Found]);
        Assert.Equal(
            [OnboardingStep.Welcome, OnboardingStep.Permissions, OnboardingStep.Models, OnboardingStep.ImportData, OnboardingStep.Polish, OnboardingStep.Ready],
            onboarding.ShownSteps);
        Assert.Equal("Step 5 of 6", onboarding.StepLabel);
        onboarding.Back();
        Assert.Equal(OnboardingStep.ImportData, onboarding.Step);
        Assert.Equal("Step 4 of 6", onboarding.StepLabel);
        Assert.Equal(Import02Model.NotNow, onboarding.NextTitle);
        onboarding.Back();
        Assert.Equal(OnboardingStep.Models, onboarding.Step);
        onboarding.Next();
        onboarding.Next();
        Assert.Equal(OnboardingStep.Polish, onboarding.Step); // Not now moves on
        onboarding.Back();
        screens.Apply([Finished()]);
        Assert.Equal("Continue", onboarding.NextTitle); // something came over
    }

    [Fact]
    public void TheFirstRunLooksForTheDataOnceItShows()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send, log: new Logged().Log);
        screens.Apply([Ev.Of("""{"type":"setting.value","key":"onboarding.done","value":"true"}""")]);
        Assert.DoesNotContain(new CoreCommand.ImportCheck(), sent.Commands); // a first run already done does not look

        var first = new Sent();
        var fresh = new ScreenModels(first.Send, log: new Logged().Log);
        fresh.Apply([Ev.Of("""{"type":"setting.value","key":"onboarding.done"}""")]);
        fresh.Apply([Ev.Of("""{"type":"setting.value","key":"dictation.polish","value":"off"}""")]);
        Assert.Single(first.Commands, c => c is CoreCommand.ImportCheck);
    }

    [Fact]
    public void AnImportReadsTheKeyNoteTheKeyAndTheLibraryAgain()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send, log: new Logged().Log);
        screens.Apply([Finished()]);
        Assert.Contains(new CoreCommand.ImportNotes(), sent.Commands); // the key note appears
        Assert.Contains(new CoreCommand.SettingGet(ShellSetting.DictationKey), sent.Commands);
        Assert.Contains(sent.Commands, c => c is CoreCommand.RecordsList); // the Library lists again
        Assert.NotNull(screens.Import02.Imported);
        foreach (var command in new[] { "import.check", "import.run" })
        {
            Assert.True(screens.Handles(Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"{{command}}","id":"{{command}}","message":"x"}""")), command);
        }
    }

    /// <summary>
    /// Settings may have read its lists before the import (Import pressed in Settings > Voice): they
    /// are read again, so an edit afterwards keeps what came over instead of saving the old list
    /// over it (the user's own list wins over the import's in the core).
    /// </summary>
    [Fact]
    public void AnImportReadsTheListsSettingsShowsAgain()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send, log: new Logged().Log);
        screens.Snippets.Load();
        screens.Apply([Ev.Of("""{"type":"snippets.listed","from_import":false,"ref":"snippets:1","snippets":[]}""")]);
        sent.Commands.Clear();

        screens.Apply([Finished()]);
        Assert.Contains(new CoreCommand.SnippetsList("snippets:2"), sent.Commands);
        Assert.Contains(new CoreCommand.VoiceCommandsList("voice_commands:1"), sent.Commands);
        Assert.Contains(new CoreCommand.ModesList(), sent.Commands);

        screens.Apply([Ev.Of("""{"type":"snippets.listed","from_import":true,"ref":"snippets:2","snippets":[{"id":"s1","trigger":"my sig","expansion":"Kind regards","category":"","enabled":true}]}""")]);
        Assert.True(screens.Snippets.FromImport);
        screens.Snippets.Add("brb", "be right back", "");
        var saved = Assert.IsType<CoreCommand.SnippetsSave>(sent.Commands[^1]);
        Assert.Equal(["my sig", "brb"], saved.Snippets.Select(s => s.Trigger)); // what came over is kept
    }
}
