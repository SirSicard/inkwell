// The first-run state (the Mac's OnboardingModelTests), and the sheet's rule that polish is never
// turned on by it without the consent step.
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
        foreach (var _ in Enum.GetValues<OnboardingStep>())
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
        Assert.Equal("Step 1 of 4", onboarding.StepLabel);
        Assert.True(onboarding.ShowsSkip);
        Assert.False(onboarding.ShowsBack);
        Assert.Equal("Continue", onboarding.NextTitle);
        onboarding.Next();
        onboarding.Next();
        onboarding.Next();
        Assert.Equal(OnboardingStep.Ready, onboarding.Step);
        Assert.Equal("Step 4 of 4", onboarding.StepLabel);
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
}
