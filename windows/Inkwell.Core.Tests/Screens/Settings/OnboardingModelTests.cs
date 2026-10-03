// The first-run state (the Mac's OnboardingModelTests), the sheet's rule that polish is never
// turned on by it without the consent step, and its models step: nothing downloads until its
// Download, and Continue never waits for one.
using System.Globalization;
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.SettingsEvents;

namespace Inkwell.Core.Tests.Screens;

public class OnboardingModelTests
{
    /// <summary>
    /// Quitting ends the first-run sheet, and a sheet ended that way is neither completed nor
    /// skipped: the next launch shows it again.
    /// </summary>
    [Fact]
    public void ASheetEndedByQuittingIsNotRecordedAsSkipped()
    {
        var sent = new Sent();
        var onboarding = new OnboardingModel(sent.Send);
        onboarding.Load();
        onboarding.Apply(Ev.Of("""{"type":"setting.value","key":"onboarding.done"}"""));
        Assert.True(onboarding.Showing);
        onboarding.AppQuitting();
        Assert.False(onboarding.Showing); // the sheet goes, so the app can quit
        // The window reports the sheet dismissed: that is not the user skipping it.
        onboarding.SheetDismissed();
        Assert.Equal([new CoreCommand.SettingGet(ShellSetting.OnboardingDone)], sent.Commands); // nothing recorded
        Assert.False(onboarding.Completed); // still not completed: shown at the next launch

        // Dismissed by the user (Escape) while the app runs: skipped, as before.
        var skipped = new OnboardingModel(sent.Send);
        skipped.Apply(Ev.Of("""{"type":"setting.value","key":"onboarding.done"}"""));
        skipped.SheetDismissed();
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.OnboardingDone, "true"), sent.Commands[^1]);
        Assert.False(skipped.Showing);
    }

    [Fact]
    public void AFirstRunStateThatCannotBeReadIsShownAndLogged()
    {
        var logged = new Logged();
        var sent = new Sent();
        var onboarding = new OnboardingModel(sent.Send, logged.Log);
        onboarding.Load();
        Assert.Equal([new CoreCommand.SettingGet(ShellSetting.OnboardingDone)], sent.Commands);
        Assert.Equal("setting:onboarding.done", JsonDocument.Parse(sent.Commands[0].Json).RootElement.GetProperty("id").GetString());
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.get","id":"setting:onboarding.done","message":"the library could not be read"}""");
        Assert.True(OnboardingModel.Handles(failed));
        onboarding.Apply(failed);
        Assert.True(onboarding.Showing); // fails toward showing it
        Assert.Single(logged.Messages);
        Assert.Contains("setting.get", logged.Messages[0], StringComparison.Ordinal);
        // Another setting's failure is not the first run's.
        var other = new OnboardingModel(_ => { }, logged.Log);
        other.Apply(Ev.Of("""{"type":"command.failed","command":"setting.get","id":"setting:dictation.polish","message":"x"}"""));
        Assert.False(other.Showing);
        // A failed write is not shown (the first run shows again next launch): it is logged.
        Assert.False(OnboardingModel.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:onboarding.done","message":"x"}""")));
    }

    [Fact]
    public void TheFirstRunStateShowsUntilItIsCompletedAndRemembersThat()
    {
        var sent = new Sent();
        var onboarding = new OnboardingModel(sent.Send);
        Assert.False(onboarding.Showing); // nothing until the store answers
        onboarding.Load();
        onboarding.Apply(Ev.Of("""{"type":"setting.value","key":"onboarding.done"}"""));
        Assert.True(onboarding.Showing); // never completed
        foreach (var _ in onboarding.ShownSteps)
        {
            onboarding.Next();
        }
        Assert.False(onboarding.Showing);
        Assert.Equal(
            [new CoreCommand.SettingGet(ShellSetting.OnboardingDone), new CoreCommand.SettingSet(ShellSetting.OnboardingDone, "true")],
            sent.Commands);
        var again = new OnboardingModel(_ => { });
        again.Apply(Ev.Of("""{"type":"setting.value","key":"onboarding.done","value":"true"}"""));
        Assert.False(again.Showing);
    }

    /// <summary>The sheet's buttons and dots, as OnboardingView shows them.</summary>
    [Fact]
    public void TheSheetsButtonsFollowTheStep()
    {
        var onboarding = new OnboardingModel(_ => { });
        Assert.Equal("Step 1 of 6", onboarding.StepLabel);
        Assert.True(onboarding.ShowsSkip);
        Assert.False(onboarding.ShowsBack);
        Assert.Equal("Continue", onboarding.NextTitle);
        onboarding.Next();
        onboarding.Next();
        Assert.Equal(OnboardingStep.Models, onboarding.Step); // after the permissions
        Assert.Equal("Continue", onboarding.NextTitle);
        onboarding.Next();
        Assert.Equal(OnboardingStep.Appearance, onboarding.Step); // after the models
        onboarding.Next();
        onboarding.Next();
        Assert.Equal(OnboardingStep.Ready, onboarding.Step);
        Assert.Equal("Step 6 of 6", onboarding.StepLabel);
        Assert.False(onboarding.ShowsSkip);
        Assert.True(onboarding.ShowsBack);
        Assert.Equal("Start", onboarding.NextTitle);
        onboarding.Back();
        Assert.Equal(OnboardingStep.Polish, onboarding.Step);
        Assert.Equal("Hold Right Ctrl and speak: your words are typed where your cursor is.",
            OnboardingModel.WelcomeLines(DictationModel.Key(DictationModel.DefaultKey)!.Name)[0]);
        Assert.Contains("on this PC", OnboardingModel.WelcomeLines("Right Ctrl")[2], StringComparison.Ordinal);
    }

    /// <summary>
    /// Owner decision: no path turns polish on without the consent step. The sheet's switch only
    /// asks (on the sheet, not Settings); finishing the first run sends nothing that turns it on.
    /// </summary>
    [Fact]
    public void TheFirstRunNeverTurnsPolishOnWithoutTheConsentStep()
    {
        var sent = new Sent();
        var screens = new SettingsHarness(sent.Send);
        screens.Apply(
            Ev.Of("""{"type":"core.ready","abi":2,"version":"0.0.0"}"""),
            Ev.Of("""{"type":"setting.value","key":"onboarding.done"}"""),
            Ev.Of(LocalLlm),
            State(on: false, allowed: false));
        var onboarding = screens.Onboarding;
        onboarding.Next();
        onboarding.Next();
        onboarding.Next();
        onboarding.Next();
        Assert.Equal(OnboardingStep.Polish, onboarding.Step);
        screens.Polish.SetOn(true, ConsentHost.Onboarding);
        Assert.True(screens.Polish.Consent.IsShowingStep(ConsentHost.Onboarding));
        Assert.False(screens.Polish.Consent.IsShowingStep(ConsentHost.Settings)); // the sheet's step, not Settings'
        onboarding.Next();
        onboarding.Next();
        Assert.False(onboarding.Showing);
        Assert.DoesNotContain(sent.Commands, c => c is CoreCommand.ConsentAllow);
        Assert.DoesNotContain(sent.Commands, c => c is CoreCommand.SettingSet { Key: ShellSetting.DictationPolish, Value: not "off" });
        Assert.False(screens.Polish.IsOn);
    }

    /// <summary>The ready step names the cards still off, and only those.</summary>
    [Fact]
    public void TheReadyStepNamesTheCardsStillOff()
    {
        var permissions = new PermissionsModel(_ => { });
        permissions.Refresh();
        permissions.Apply(Ev.Of("""{"type":"permissions.checked","microphone":"granted","system_audio":"granted","accessibility":"granted","input_monitoring":"granted"}"""));
        Assert.Null(OnboardingModel.StillOff(permissions)); // the calendar not available is not off
        permissions.Apply(Ev.Of("""{"type":"permissions.checked","microphone":"denied","system_audio":"granted","accessibility":"granted","input_monitoring":"granted"}"""));
        Assert.Equal("Still off: Hear you. Settings can turn them on.", OnboardingModel.StillOff(permissions));
    }

    /// <summary>
    /// Windows' recommended set (the Mac's 28e5107, Windows' version): the step's one Download
    /// fetches only Silero VAD and Parakeet TDT v3, smallest first, saying how much and from where;
    /// Qwen3-ASR and the diarizer are optional, each with a Download of its own. It says plainly
    /// that a meeting keeps its live transcript until the meeting model is added, because Windows'
    /// Parakeet has no final pass for meetings.
    /// </summary>
    [Fact]
    public void TheStepsDownloadIsTheRecommendedSetAndTheMeetingModelIsOptional()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(CatalogueDownloadTests.ListedWindows());
        Assert.Equal(
            "Inkwell turns speech into text with models that run on this PC. The recommended two are small: Silero VAD hears when you speak, and Parakeet TDT v3 types your dictation and writes a meeting's transcript as it happens.",
            OnboardingModel.ModelsNote(catalogue));
        Assert.Equal(
            "640 MB in all, from huggingface.co and raw.githubusercontent.com. Nothing downloads until you press Download.",
            OnboardingModel.DownloadLine(catalogue, CultureInfo.InvariantCulture));
        Assert.Equal(
            "Download Parakeet TDT v3 and Silero VAD: 640 MB in all, from huggingface.co and raw.githubusercontent.com",
            OnboardingModel.DownloadName(catalogue, CultureInfo.InvariantCulture));
        Assert.Equal(
            "On Windows, a meeting keeps that live transcript until you add the meeting model, Qwen3-ASR (about 2.5 GB): Parakeet has no final pass for meetings here. You can add it below, or later in Settings > Models.",
            OnboardingModel.MeetingModelNote(catalogue));
        Assert.True(CatalogueModel.IsRecommended("parakeet-tdt-0.6b-v3-int8"));
        Assert.False(CatalogueModel.IsRecommended("qwen3-asr-1.7b-q8"));

        catalogue.DownloadRecommended();
        Assert.Equal(["silero-vad-v6-16k"], sent.Commands.OfType<CoreCommand.ModelUpdate>().Select(u => u.Next)); // the smallest first
        catalogue.Apply(CatalogueDownloadTests.Finished("silero-vad-v6-16k", ok: true));
        Assert.Equal(["silero-vad-v6-16k", "parakeet-tdt-0.6b-v3-int8"], sent.Commands.OfType<CoreCommand.ModelUpdate>().Select(u => u.Next));
        catalogue.Apply(CatalogueDownloadTests.Finished("parakeet-tdt-0.6b-v3-int8", ok: true));
        Assert.Equal(2, sent.Commands.OfType<CoreCommand.ModelUpdate>().Count()); // Qwen3-ASR and the diarizer only on their own Download
        Assert.Null(OnboardingModel.DownloadLine(catalogue, CultureInfo.InvariantCulture));

        catalogue.Apply(CatalogueDownloadTests.ListedWindows(qwen: true, parakeet: true, silero: true));
        Assert.Null(OnboardingModel.MeetingModelNote(catalogue)); // the meeting model is in
    }

    /// <summary>
    /// The models step names each model not installed (size, licence), how much in all and from
    /// where; going through the whole first run without pressing its Download downloads nothing.
    /// </summary>
    [Fact]
    public void TheModelsStepNamesWhatIsMissingAndNothingDownloadsWithoutItsButton()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send, log: new Logged().Log);
        screens.Apply([Ev.Of("""{"type":"core.ready","abi":2,"version":"0.0.0"}"""), Ev.Of("""{"type":"setting.value","key":"onboarding.done"}""")]);
        var onboarding = screens.Onboarding;
        var catalogue = screens.Catalogue;
        Assert.Equal("Checking which models are on this PC…", OnboardingModel.ModelsNote(catalogue));
        Assert.Null(OnboardingModel.DownloadLine(catalogue, CultureInfo.InvariantCulture)); // no Download before the list
        screens.Apply([CatalogueDownloadTests.Listed()]);
        onboarding.Next();
        onboarding.Next();
        Assert.Equal(OnboardingStep.Models, onboarding.Step);
        var rows = OnboardingModel.ModelRows(catalogue);
        Assert.Equal(["silero-vad-v6-16k", "qwen3-asr-1.7b-q8"], rows.Select(r => r.Id)); // not the installed one; the recommended first
        Assert.Equal("Qwen3-ASR 1.7B · Apache-2.0 · 2.32 GB · not installed", rows[1].Text(CultureInfo.InvariantCulture));
        Assert.StartsWith("Inkwell turns speech into text with models that run on this PC.", OnboardingModel.ModelsNote(catalogue), StringComparison.Ordinal);
        Assert.Equal(
            "1.22 MB in all, from raw.githubusercontent.com. Nothing downloads until you press Download.",
            OnboardingModel.DownloadLine(catalogue, CultureInfo.InvariantCulture));
        Assert.Equal(
            "Download Silero VAD: 1.22 MB in all, from raw.githubusercontent.com",
            OnboardingModel.DownloadName(catalogue, CultureInfo.InvariantCulture));
        foreach (var _ in Enum.GetValues<OnboardingStep>())
        {
            onboarding.Next();
        }
        Assert.False(onboarding.Showing);
        Assert.DoesNotContain(sent.Commands, c => c is CoreCommand.ModelUpdate);
    }

    /// <summary>The step's Download fetches what it named, one at a time; Continue goes on while it runs, and so do the downloads.</summary>
    [Fact]
    public void ContinueGoesOnWhileTheDownloadsRun()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send, log: new Logged().Log);
        screens.Apply([Ev.Of("""{"type":"setting.value","key":"onboarding.done"}"""), CatalogueDownloadTests.Listed()]);
        var onboarding = screens.Onboarding;
        var catalogue = screens.Catalogue;
        onboarding.Next();
        onboarding.Next();
        catalogue.DownloadRecommended(); // the step's Download
        catalogue.Download("qwen3-asr-1.7b-q8"); // and the meeting model's own
        Assert.Equal(["silero-vad-v6-16k"], sent.Commands.OfType<CoreCommand.ModelUpdate>().Select(u => u.Next)); // the smallest first
        Assert.Null(OnboardingModel.DownloadLine(catalogue, CultureInfo.InvariantCulture)); // nothing left to ask for: the button goes
        Assert.True(catalogue.Downloading); // "You can go on…" shows
        onboarding.Next();
        Assert.Equal(OnboardingStep.Appearance, onboarding.Step); // Continue does not wait
        onboarding.Next();
        onboarding.Next();
        onboarding.Next();
        Assert.False(onboarding.Showing);
        screens.Apply([
            CatalogueDownloadTests.Progress("silero-vad-v6-16k", 1_289_603, 1_289_603),
            CatalogueDownloadTests.Finished("silero-vad-v6-16k", ok: true),
            CatalogueDownloadTests.Listed(sileroInstalled: true),
        ]);
        // The next one went after the sheet had gone.
        Assert.Equal(["silero-vad-v6-16k", "qwen3-asr-1.7b-q8"], sent.Commands.OfType<CoreCommand.ModelUpdate>().Select(u => u.Next));
        // A model installed this run stays on the step, as installed.
        Assert.Equal(["silero-vad-v6-16k", "qwen3-asr-1.7b-q8"], OnboardingModel.ModelRows(catalogue).Select(r => r.Id));
        Assert.StartsWith("Inkwell turns speech into text", OnboardingModel.ModelsNote(catalogue), StringComparison.Ordinal);
        screens.Apply([CatalogueDownloadTests.Finished("qwen3-asr-1.7b-q8", ok: true)]);
        Assert.Equal("Every model Inkwell uses is on this PC.", OnboardingModel.ModelsNote(catalogue));
        Assert.False(catalogue.Downloading);
    }

    [Fact]
    public void TheModelsStepSaysWhenTheListCannotBeReadOrNothingIsMissing()
    {
        var catalogue = new CatalogueModel(_ => { });
        catalogue.Apply(Ev.Of("""{"type":"command.failed","command":"models.list","message":"the registry could not be read"}"""));
        Assert.Equal(CatalogueModel.FailedText, OnboardingModel.ModelsNote(catalogue)); // with Try again
        catalogue.Apply(CatalogueDownloadTests.Listed(qwenInstalled: true, sileroInstalled: true));
        Assert.Equal("Every model Inkwell uses is on this PC.", OnboardingModel.ModelsNote(catalogue));
        Assert.Empty(OnboardingModel.ModelRows(catalogue));
        Assert.Null(OnboardingModel.DownloadLine(catalogue, CultureInfo.InvariantCulture));
    }
}
