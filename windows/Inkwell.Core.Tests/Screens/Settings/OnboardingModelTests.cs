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
    /// The models step offers its choices with the total on Download; going through the whole
    /// first run without pressing it downloads nothing.
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
        Assert.Null(onboarding.Choices.DownloadTitle(catalogue, CultureInfo.InvariantCulture)); // no Download before the list
        screens.Apply([CatalogueDownloadTests.Listed()]);
        onboarding.Next();
        onboarding.Next();
        Assert.Equal(OnboardingStep.Models, onboarding.Step);
        // This catalogue lists Silero VAD, Qwen3-ASR and the diarizer (installed), no Parakeet.
        Assert.Equal(["speech", "accuracy", "speakers"], ModelChoices.Shown(catalogue).Select(c => c.Id));
        Assert.Equal("Installed", ModelChoices.Status(ModelChoices.Speakers, catalogue, CultureInfo.InvariantCulture));
        Assert.StartsWith("Inkwell writes down speech with models that run on this PC.", OnboardingModel.ModelsNote(catalogue), StringComparison.Ordinal);
        Assert.Equal("Download 1.22 MB", onboarding.Choices.DownloadTitle(catalogue, CultureInfo.InvariantCulture));
        Assert.Equal(
            "Download Silero VAD: 1.22 MB in all, from raw.githubusercontent.com",
            onboarding.Choices.DownloadName(catalogue, CultureInfo.InvariantCulture));
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
        onboarding.Choices.Tick(ModelChoices.Accuracy, on: true); // Fewer mistakes
        onboarding.Choices.Download(catalogue); // the step's Download
        Assert.Equal(["silero-vad-v6-16k"], sent.Commands.OfType<CoreCommand.ModelUpdate>().Select(u => u.Next)); // the smallest first
        Assert.Null(onboarding.Choices.DownloadTitle(catalogue, CultureInfo.InvariantCulture)); // nothing left to ask for: the button goes
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
        // A choice installed this run says so.
        Assert.Equal("Installed", ModelChoices.Status(ModelChoices.Speech, catalogue, CultureInfo.InvariantCulture));
        Assert.StartsWith("Inkwell writes down speech", OnboardingModel.ModelsNote(catalogue), StringComparison.Ordinal);
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
        Assert.Null(new ModelChoices().DownloadTitle(catalogue, CultureInfo.InvariantCulture));
    }

    /// <summary>The own key is one choice, in the Mac's words (how to get the key: GroqKeyGuideTests).</summary>
    [Fact]
    public void TheOwnKeyIsOneChoiceInTheMacsWords()
    {
        Assert.Equal("Use Groq's free model", OnboardingModel.OwnKeyTitle);
        Assert.Equal("Paste your Groq key", OnboardingModel.OwnKeyPlaceholder);
        Assert.Equal("Save", OnboardingModel.OwnKeySave);
        Assert.Equal("Other providers or models\u2026", OnboardingModel.OtherProviders);
        Assert.Equal("Back to Groq's free model", OnboardingModel.BackToGroq);
    }


    /// <summary>With no speech model the last step never says "Hold … and speak": it says one is needed.</summary>
    [Fact]
    public void TheLastStepWithNoSpeechModelSaysOneIsNeeded()
    {
        Assert.StartsWith("Hold Right Ctrl, say something, and let go.", OnboardingModel.ReadyLine("Right Ctrl"), StringComparison.Ordinal);
        var line = OnboardingModel.ReadyLine("Right Ctrl", noSpeechModel: true);
        Assert.Equal("Inkwell needs a speech model before it can type what you say.", line);
        Assert.Equal("Inkwell lives in the notification area; this window opens from there.", OnboardingModel.TrayLine);
        Assert.EndsWith(OnboardingModel.TrayLine, OnboardingModel.ReadyLine("Right Ctrl"), StringComparison.Ordinal);
        Assert.DoesNotContain("Hold", line, StringComparison.Ordinal);
        Assert.Equal("Download recommended models", NeedsYou.DownloadModelsTitle);
    }


    private sealed class HeldWakes : IWakeScheduler
    {
        public List<Action> Waiting { get; } = [];

        public IDisposable After(TimeSpan delay, Action wake)
        {
            Assert.Equal(OnboardingModel.NotHearingAfter, delay);
            Waiting.Add(wake);
            return new Cancel(() => Waiting.Remove(wake));
        }

        private sealed class Cancel(Action cancel) : IDisposable
        {
            public void Dispose() => cancel();
        }
    }

    /// <summary>
    /// The Ready step's hint (as the Mac's TryItHint): after a take that heard no speech or only
    /// silence, or 5 s into a held take with no live words (only where a live model gives words);
    /// gone when words come or a take types something.
    /// </summary>
    [Fact]
    public void TheTryItSaysWhereToPickTheMicrophoneWhenItHearsNothing()
    {
        var wakes = new HeldWakes();
        var live = true;
        var onboarding = new OnboardingModel(new Sent().Send) { Wake = wakes, HasLiveWords = () => live };
        onboarding.Apply(Ev.Of("""{"type":"setting.value","key":"onboarding.done"}"""));
        // On another step a take is not the try-it: no look, no hint.
        onboarding.Apply(Ev.Of("""{"type":"dictation.started","take":0,"edit":false}"""));
        onboarding.Apply(Ev.Of("""{"type":"dictation.discarded","reason":"no_speech"}"""));
        Assert.Empty(wakes.Waiting);
        Assert.False(onboarding.NotHearing);
        while (onboarding.Step != OnboardingStep.Ready)
        {
            onboarding.Next();
        }
        onboarding.Apply(Ev.Of("""{"type":"dictation.started","take":1,"edit":false}"""));
        Assert.Single(wakes.Waiting);
        wakes.Waiting[0]();
        Assert.True(onboarding.NotHearing);
        Assert.Contains("Settings > Sound", OnboardingModel.NotHearingText, StringComparison.Ordinal);
        onboarding.Apply(Ev.Of("""{"type":"dictation.partial","take":1,"text":"so"}"""));
        Assert.False(onboarding.NotHearing); // words after a pause
        onboarding.Apply(Ev.Of("""{"type":"dictation.stopped"}"""));
        Assert.Empty(wakes.Waiting); // the look goes with the take

        // A take that heard words in time: the look finds them.
        onboarding.Apply(Ev.Of("""{"type":"dictation.started","take":2,"edit":false}"""));
        onboarding.Apply(Ev.Of("""{"type":"dictation.partial","take":2,"text":"hello"}"""));
        wakes.Waiting[0]();
        Assert.False(onboarding.NotHearing);
        onboarding.Apply(Ev.Of("""{"type":"dictation.discarded","reason":"no_speech"}"""));
        Assert.True(onboarding.NotHearing);
        onboarding.Apply(Ev.Of("""{"type":"dictation.inserted","text":"hi there","outcome":"pasted"}"""));
        Assert.False(onboarding.NotHearing); // it heard you

        // No live model: a held take shows no words until it ends, so the 5 s says nothing.
        live = false;
        onboarding.Apply(Ev.Of("""{"type":"dictation.started","take":4,"edit":false}"""));
        wakes.Waiting[^1]();
        Assert.False(onboarding.NotHearing);
        onboarding.Apply(Ev.Of("""{"type":"dictation.discarded","reason":"too_short"}"""));
        Assert.False(onboarding.NotHearing); // a tap says nothing about the mic
        // A voice edit is not the try-it.
        onboarding.Apply(Ev.Of("""{"type":"dictation.started","take":5,"edit":true}"""));
        Assert.Empty(wakes.Waiting);
        // Once the first run is done, takes are none of its business.
        onboarding.Apply(Ev.Of("""{"type":"dictation.discarded","reason":"silence"}"""));
        Assert.True(onboarding.NotHearing);
        onboarding.Next(); // Start
        onboarding.Apply(Ev.Of("""{"type":"dictation.started","take":6,"edit":false}"""));
        Assert.Empty(wakes.Waiting);
    }
}
