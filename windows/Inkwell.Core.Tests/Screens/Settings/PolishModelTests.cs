// The Polish toggle (the Mac's PolishModelTests, PolishSettingFailureTests and
// PolishTimeoutPathTests). Windows has no Apple Intelligence: the model is a synthetic one the core
// registers, named by consent.state.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.SettingsEvents;

namespace Inkwell.Core.Tests.Screens;

public class PolishModelTests
{
    private static CoreCommand.ConsentGet Get(int n) => new CoreCommand.ConsentGet(LlmFeature.Polish, $"consent.get:polish:{n}");

    /// <summary>
    /// Verify: the Polish toggle can't read "on" without a working engine. Windows: the Mac's
    /// Apple Intelligence reasons are gone; with no language model the line says there is none.
    /// </summary>
    [Fact]
    public void TheToggleNeverReadsOnWithoutAWorkingEngine()
    {
        var sent = new Sent();
        var polish = new PolishModel(sent.Send);
        polish.Apply(State(on: true, allowed: true, allowedTo: "on_device"));
        Assert.True(polish.Preference);
        Assert.False(polish.IsOn); // wished for, but nothing can polish
        Assert.False(polish.CanToggle);
        polish.SetOn(true);
        Assert.Null(polish.PendingConsent); // a toggle with no engine asks nothing
        Assert.Empty(sent.Commands); // and sends nothing
        Assert.Equal(PolishModel.NoModelText, polish.Status);

        polish.Apply(Ev.Of(LocalLlm));
        Assert.True(polish.IsOn);
        Assert.True(polish.CanToggle);
        Assert.Equal([Get(1)], sent.Commands); // a new model: where polish sends is read again

        // A speech engine is not a language model.
        polish.Apply(Ev.Of("""{"type":"engine.unregistered","id":"local-llm"}"""));
        polish.Apply(Ev.Of("""{"type":"engine.registered","id":"parakeet","kind":"streaming","jobs":[{"job":"live_partials","wer":21.3}]}"""));
        Assert.False(polish.IsOn); // let go of when the model went away

        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(Ev.Of("""{"type":"core.stopped"}"""));
        Assert.False(polish.IsOn); // a stopped core holds no engine
    }

    /// <summary>
    /// Owner decision: turning polish on asks first, naming where the words go; Cancel sends
    /// nothing and leaves it off; Allow asks the core to record the consent.
    /// </summary>
    [Fact]
    public void TurningItOnAsksFirstAndCancelLeavesItOff()
    {
        var sent = new Sent();
        var polish = new PolishModel(sent.Send);
        polish.Load();
        Assert.Equal([Get(1)], sent.Commands);
        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(State(on: false, allowed: false));
        Assert.False(polish.IsOn);
        Assert.Equal("Off. Your words go in as you said them.", polish.Status);

        polish.SetOn(true);
        var asked = Assert.IsType<ConsentDestination>(polish.PendingConsent);
        Assert.Equal("Example Local Model, on this PC", asked.Label);
        Assert.Equal([Get(1), Get(2)], sent.Commands); // asking sends nothing
        var message = PolishModel.ConsentMessage(asked);
        Assert.Contains("sends what you dictate to a language model", message, StringComparison.Ordinal);
        Assert.Contains("Example Local Model, on this PC, so your words stay on this PC", message, StringComparison.Ordinal);
        Assert.DoesNotContain("invisible", message, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("undetectable", message, StringComparison.OrdinalIgnoreCase);
        Assert.Equal("Turn on polish?", PolishModel.ConsentTitle);
        Assert.Equal("Turn On Polish", PolishModel.ConsentButton(asked));

        polish.CancelConsent();
        Assert.Null(polish.PendingConsent);
        Assert.False(polish.IsOn); // cancel leaves it off
        Assert.Equal([Get(1), Get(2)], sent.Commands); // and sends nothing

        polish.SetOn(true);
        polish.AllowConsent();
        Assert.Equal(new CoreCommand.ConsentAllow(LlmFeature.Polish, LlmDestination.OnDevice, null, null, "consent.allow:polish:3"), sent.Commands[^1]);
        Assert.False(polish.IsOn); // on only once the core has recorded it
        polish.Apply(State(on: true, allowed: true, allowedTo: "on_device"));
        Assert.True(polish.IsOn);
        Assert.Equal("On, with Example Local Model, on this PC. Your words stay on this PC.", polish.Status);
    }

    /// <summary>Windows: a model the core names no name for is "the on-device model on this PC", never a made-up name.</summary>
    [Fact]
    public void AnUnnamedOnDeviceModelIsCalledTheOnDeviceModelOnThisPc()
    {
        var polish = new PolishModel(_ => { });
        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(State(on: false, allowed: false, name: ""));
        polish.SetOn(true);
        var asked = Assert.IsType<ConsentDestination>(polish.PendingConsent);
        Assert.Equal("the on-device model on this PC", asked.Label);
        Assert.EndsWith("It uses the on-device model on this PC, so your words stay on this PC.", PolishModel.ConsentMessage(asked), StringComparison.Ordinal);
    }

    /// <summary>A cloud model is named, and the step says the words leave this PC for it.</summary>
    [Fact]
    public void ACloudModelIsNamedAndTheStepSaysTheWordsLeaveThisMac()
    {
        var sent = new Sent();
        var polish = new PolishModel(sent.Send);
        polish.Apply(Ev.Of("""{"type":"engine.registered","id":"cloud","kind":"llm","jobs":[]}"""));
        polish.Apply(State(on: false, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud"));
        polish.SetOn(true);
        var asked = Assert.IsType<ConsentDestination>(polish.PendingConsent);
        var message = PolishModel.ConsentMessage(asked);
        Assert.Contains("your words leave this PC and go to Example Cloud", message, StringComparison.Ordinal);
        Assert.Equal("Send to Example Cloud", PolishModel.ConsentButton(asked));
        polish.AllowConsent();
        Assert.Equal(new CoreCommand.ConsentAllow(LlmFeature.Polish, LlmDestination.Cloud, "shell engine cloud", null, "consent.allow:polish:2"), sent.Commands[^1]);
        polish.Apply(State(on: true, allowed: true, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", allowedTo: "cloud"));
        Assert.Equal("On. Your words go to Example Cloud before they are typed.", polish.Status);
    }

    /// <summary>
    /// The model moves from this PC to a cloud provider while polish is on: it reads off, says
    /// why, and turning it on asks about the new destination.
    /// </summary>
    [Fact]
    public void AModelThatMovesPausesPolishAndSaysWhy()
    {
        var polish = new PolishModel(_ => { });
        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(State(on: true, allowed: true, allowedTo: "on_device"));
        Assert.True(polish.IsOn);
        polish.Apply(Ev.Of("""{"type":"engine.registered","id":"cloud","kind":"llm","jobs":[]}"""));
        polish.Apply(State(on: true, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", allowedTo: "on_device"));
        Assert.False(polish.IsOn); // nothing is polished
        Assert.True(polish.IsPaused);
        Assert.True(polish.IsProblem);
        Assert.Equal("Paused: polish would now send your words to Example Cloud. Turn it on again to allow it.", polish.Status);
        polish.SetOn(true);
        Assert.Equal(ConsentDestination.Cloud("shell engine cloud", "Example Cloud"), polish.PendingConsent);
        // A consent that could not be read is none, and says so.
        polish.CancelConsent();
        polish.Apply(State(on: true, allowed: false, allowedTo: null, error: "couldn't read your polish consent"));
        Assert.Equal("Couldn't read your polish consent, so polish is off or paused. Turn it on again to allow it.", polish.Status);
    }

    /// <summary>
    /// The step asked about one destination and the model moved before Allow: the step closes
    /// (its words are never swapped under the user), nothing is sent, and asking again names the
    /// new destination.
    /// </summary>
    [Fact]
    public void TheStepClosesIfTheModelMovesWhileItIsShown()
    {
        var sent = new Sent();
        var polish = new PolishModel(sent.Send);
        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(State(on: false, allowed: false));
        polish.SetOn(true, ConsentHost.Onboarding);
        Assert.Equal(ConsentHost.Onboarding, polish.ConsentHost); // shown where it was asked
        Assert.True(polish.Consent.IsShowingStep(ConsentHost.Onboarding));
        Assert.False(polish.Consent.IsShowingStep(ConsentHost.Settings));
        polish.Apply(State(on: false, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud"));
        Assert.Null(polish.PendingConsent);
        Assert.Null(polish.ConsentHost);
        polish.AllowConsent();
        Assert.DoesNotContain(sent.Commands, c => c is CoreCommand.ConsentAllow); // nothing sent
        polish.SetOn(true);
        Assert.Equal("Example Cloud", polish.PendingConsent?.Name);
        Assert.Equal(ConsentHost.Settings, polish.ConsentHost);
    }

    [Fact]
    public void SwitchingOffSendsOffAndAFailedSavePutsItBack()
    {
        var sent = new Sent();
        var polish = new PolishModel(sent.Send);
        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(State(on: true, allowed: true, allowedTo: "on_device"));
        polish.SetOn(false);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.DictationPolish, "off"), sent.Commands[^1]);
        Assert.False(polish.IsOn);
        polish.Apply(Ev.Of("""{"type":"command.failed","command":"setting.set","id":"setting:dictation.polish","message":"x"}"""));
        Assert.True(polish.IsOn); // put back
        Assert.Equal("Couldn't save the change, so polish stays as it was.", polish.Status);
        // A consent the core could not record says so.
        polish.Apply(State(on: false, allowed: false));
        polish.SetOn(true);
        polish.AllowConsent();
        polish.Apply(Ev.Of("""{"type":"command.failed","command":"consent.allow","id":"consent.allow:polish:2","message":"x"}"""));
        Assert.Equal("Couldn't turn polish on, so it stays off. Try again.", polish.Status);
    }

    /// <summary>The §6 point: a timeout reads apart from an ordinary cancel or failure.</summary>
    [Fact]
    public void PolishThatKeepsTimingOutSaysSoAndAPlainFailureDoesNot()
    {
        var polish = new PolishModel(_ => { });
        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(State(on: true, allowed: true, allowedTo: "on_device"));
        void Take(string? warning)
        {
            polish.Apply(Ev.Of("""{"type":"dictation.started","take":0,"edit":false}"""));
            if (warning is not null)
            {
                polish.Apply(Ev.Of(warning));
            }
            polish.Apply(Ev.Of("""{"type":"dictation.inserted","text":"x","outcome":"pasted"}"""));
        }
        Take("""{"type":"dictation.warning","kind":"polish_failed","message":"cancelled"}""");
        Take("""{"type":"dictation.warning","kind":"polish_failed","message":"cancelled"}""");
        Assert.False(polish.KeepsTimingOut); // a cancel is not a timeout
        Take("""{"type":"dictation.warning","kind":"polish_timed_out"}""");
        Assert.False(polish.KeepsTimingOut); // once is not keeps
        Take("""{"type":"dictation.warning","kind":"polish_timed_out"}""");
        Assert.True(polish.KeepsTimingOut);
        Assert.Equal("Polish keeps timing out, so your words go in as you said them.", polish.Status);
        Assert.True(polish.IsOn); // still on: the engine works, it is slow
        Take(null);
        Assert.False(polish.KeepsTimingOut); // a take that polished in time ends the run
    }

    /// <summary>
    /// Answers can arrive out of order: only the answer to the newest request is applied, and a
    /// failure counts only for the request that is still the newest of its kind.
    /// </summary>
    [Fact]
    public void OnlyTheAnswerToTheNewestRequestIsApplied()
    {
        var sent = new Sent();
        var polish = new PolishModel(sent.Send);
        polish.Apply(Ev.Of(LocalLlm)); // consent.get:polish:1
        polish.Load();                 // consent.get:polish:2
        Assert.Equal([Get(1), Get(2)], sent.Commands);
        polish.Apply(State(on: true, allowed: true, reference: "consent.get:polish:2"));
        Assert.True(polish.IsOn);
        polish.Apply(State(on: false, allowed: false, reference: "consent.get:polish:1"));
        Assert.True(polish.IsOn); // the older answer, arriving late, is not applied
        polish.Apply(Ev.Of("""{"type":"command.failed","command":"consent.get","id":"consent.get:polish:1","message":"x"}"""));
        Assert.Null(polish.Failure); // nor is an older request's failure
        // A state with no ref (the core's own, after a switch-off) always applies.
        polish.Apply(State(on: false, allowed: false));
        Assert.False(polish.IsOn);
    }

    /// <summary>
    /// A part of the state the core could not read is said whenever it is there: with polish off
    /// (an unread switch reads as off in the core) and with no working model.
    /// </summary>
    [Fact]
    public void AReadErrorIsShownWhetherOrNotPolishIsOn()
    {
        var polish = new PolishModel(_ => { });
        polish.Apply(State(on: false, allowed: false, error: "couldn't read the switch"));
        Assert.Equal("Couldn't read the switch, so polish is off or paused. Turn it on again to allow it.", polish.Status);
        Assert.True(polish.IsProblem);
        polish.Apply(Ev.Of(LocalLlm));
        Assert.Equal("Couldn't read the switch, so polish is off or paused. Turn it on again to allow it.", polish.Status);
        polish.Apply(State(on: false, allowed: false));
        Assert.Equal("Off. Your words go in as you said them.", polish.Status);
    }

    /// <summary>
    /// A take the core refused to polish makes the toggle read the state again. (The Drop's "Not
    /// polished" note is the Drop's, not ported here.)
    /// </summary>
    [Fact]
    public void ATakeRefusedForConsentSaysSo()
    {
        var sent = new Sent();
        var polish = new PolishModel(sent.Send);
        polish.Apply(Ev.Of("""{"type":"dictation.warning","kind":"polish_not_allowed","message":"Example Cloud"}"""));
        Assert.Equal([Get(1)], sent.Commands);
    }

    /// <summary>A view bound to the toggle hears about a change in its consent.</summary>
    [Fact]
    public void AConsentChangeIsAChangeOfTheToggle()
    {
        var polish = new PolishModel(_ => { });
        var before = polish.Changes;
        polish.Apply(State(on: true, allowed: true));
        Assert.True(polish.Changes > before);
    }
}

public class PolishSettingFailureTests
{
    /// <summary>§6 carried item: a failed polish read or write gets UI, not only the log.</summary>
    [Fact]
    public void APolishSettingThatCouldNotBeReadOrSavedSaysSoAndTheToggleGoesBack()
    {
        var polish = new PolishModel(_ => { });
        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(Ev.Of("""{"type":"command.failed","command":"consent.get","id":"consent.get:polish:1","message":"the library could not be read"}"""));
        Assert.Equal(ConsentFailure.Read, polish.Failure);
        Assert.Equal("Couldn't read your polish setting. Open Settings again to retry.", polish.Status);
        Assert.False(polish.CanToggle); // unknown, so not switchable
        Assert.True(polish.IsProblem);
        polish.Apply(State(on: true, allowed: true, allowedTo: "on_device"));
        Assert.Null(polish.Failure);
        Assert.True(polish.IsOn);
        polish.SetOn(false);
        Assert.False(polish.IsOn);
        polish.Apply(Ev.Of("""{"type":"command.failed","command":"setting.set","id":"setting:dictation.polish","message":"the library could not be written"}"""));
        Assert.Equal(ConsentFailure.Write, polish.Failure);
        Assert.True(polish.IsOn); // back where it was: the change was not saved
        Assert.Equal("Couldn't save the change, so polish stays as it was.", polish.Status);
        // The screens show it, so the aggregator does not log it as unshown.
        var screens = new SettingsHarness(_ => { });
        Assert.True(screens.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:dictation.polish","message":"x"}""")));
        Assert.True(screens.Polish.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"consent.get","id":"consent.get:polish:4","message":"x"}""")));
        Assert.False(screens.Polish.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"consent.get","id":"consent.get:edit:1","message":"x"}""")));
    }
}

public class PolishTimeoutPathTests
{
    [Fact]
    public void TheCoresTimeoutEventMakesTheToggleSayItKeepsTimingOut()
    {
        var timedOut = Ev.Of<DictationWarningEvent>("""{"type":"dictation.warning","kind":"polish_timed_out"}""");
        Assert.Equal(DictationWarning.PolishTimedOut, timedOut.Kind);
        Assert.Null(timedOut.Message); // no text

        var screens = new SettingsHarness(_ => { });
        screens.Apply(Ev.Of(LocalLlm), State(on: true, allowed: true, allowedTo: "on_device"));
        for (var i = 0; i < PolishModel.TimeoutWarning; i++)
        {
            screens.Apply(
                Ev.Of("""{"type":"dictation.started","take":0,"edit":false}"""), timedOut,
                Ev.Of("""{"type":"dictation.inserted","text":"x","outcome":"pasted"}"""));
        }
        Assert.True(screens.Polish.KeepsTimingOut);
        Assert.Equal("Polish keeps timing out, so your words go in as you said them.", screens.Polish.Status);
    }
}
