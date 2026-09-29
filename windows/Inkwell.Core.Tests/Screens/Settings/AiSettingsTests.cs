// Voice edit's consent (the edit-key picker) and summaries and Ask's (Settings > AI): the Mac's
// EditConsentTests and MeetingsConsentTests, through AiSettings (the Mac's ScreenModels methods).
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.SettingsEvents;

namespace Inkwell.Core.Tests.Screens;

public class EditConsentTests
{
    private static SettingsHarness Screens(Sent sent)
    {
        var screens = new SettingsHarness(sent.Send);
        screens.Apply(Ev.Of(LocalLlm), State(on: false, allowed: false, feature: "edit"));
        return screens;
    }

    /// <summary>
    /// Picking an edit key from Off asks first, naming where the selection goes; Cancel sends
    /// nothing; Allow sends the consent with the key.
    /// </summary>
    [Fact]
    public void PickingAKeyAsksFirstAndCancelChangesNothing()
    {
        var sent = new Sent();
        var screens = Screens(sent);
        var before = sent.Commands.Count;
        screens.Ai.ChooseEditKey("right_alt");
        var asked = Assert.IsType<ConsentDestination>(screens.EditConsent.Pending);
        Assert.Equal(ConsentHost.Settings, screens.EditConsent.Host);
        Assert.Equal("right_alt", screens.EditConsent.PendingKey);
        Assert.Equal(before, sent.Commands.Count); // asking sends nothing
        var message = ConsentModel.Message(LlmFeature.Edit, asked);
        Assert.Contains("sends the text you select and what you say to a language model", message, StringComparison.Ordinal);
        Assert.Contains("Example Local Model, on this PC, so your words stay on this PC", message, StringComparison.Ordinal);
        Assert.Equal("Turn On Voice Edit", ConsentModel.Button(LlmFeature.Edit, asked));
        Assert.Null(screens.Dictation.EditKey); // asking changes no key
        screens.EditConsent.Cancel();
        Assert.Null(screens.EditConsent.Pending);
        Assert.Equal(before, sent.Commands.Count); // cancel sends nothing
        Assert.Null(screens.Dictation.EditKey); // cancel leaves voice edit off

        screens.Ai.ChooseEditKey("right_alt");
        screens.EditConsent.Allow();
        Assert.Equal(new CoreCommand.ConsentAllow(LlmFeature.Edit, LlmDestination.OnDevice, null, "right_alt", "consent.allow:edit:2"), sent.Commands[^1]);
    }

    /// <summary>On and allowed: another key only changes the key. Off sends off (the core withdraws the consent with it).</summary>
    [Fact]
    public void AKeyChangeWhileAllowedAsksNothingAndOffTurnsItOff()
    {
        var sent = new Sent();
        var screens = Screens(sent);
        screens.Apply(
            Ev.Of("""{"type":"setting.value","key":"dictation.edit_key","value":"right_alt"}"""),
            State(on: true, allowed: true, allowedTo: "on_device", feature: "edit"));
        screens.Ai.ChooseEditKey("right_shift");
        Assert.Null(screens.EditConsent.Pending);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.DictationEditKey, "right_shift"), sent.Commands[^1]);
        screens.Ai.ChooseEditKey(null);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.DictationEditKey, "off"), sent.Commands[^1]);
        Assert.False(screens.EditConsent.IsAllowedOn);
    }

    /// <summary>
    /// The model moved to a cloud provider: voice edit is paused and says so, a key pick asks about
    /// the new destination, and a refused edit reads the state again. (The Drop's "Not edited"
    /// note is the Drop's, not ported here.)
    /// </summary>
    [Fact]
    public void AMovedModelPausesVoiceEditAndAKeyPickAsksAgain()
    {
        var sent = new Sent();
        var screens = Screens(sent);
        screens.Apply(
            Ev.Of("""{"type":"setting.value","key":"dictation.edit_key","value":"right_alt"}"""),
            State(on: true, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", allowedTo: "on_device", feature: "edit"));
        Assert.Equal("Paused: voice edit would now send your words to Example Cloud. Turn it on again to allow it.", screens.Ai.EditConsentProblem);
        screens.Ai.ChooseEditKey("right_shift");
        var asked = Assert.IsType<ConsentDestination>(screens.EditConsent.Pending);
        Assert.Equal(ConsentDestination.Cloud("shell engine cloud", "Example Cloud"), asked);
        Assert.Equal("Send to Example Cloud", ConsentModel.Button(LlmFeature.Edit, asked));
        var before = sent.Commands.Count;
        screens.Apply(Ev.Of("""{"type":"dictation.edit_failed","reason":"not_allowed","message":"Example Cloud"}"""));
        Assert.Equal(new CoreCommand.ConsentGet(LlmFeature.Edit, "consent.get:edit:2"), sent.Commands[before]);
    }

    /// <summary>Polish's state never moves voice edit's, nor the reverse.</summary>
    [Fact]
    public void EachFeatureReadsOnlyItsOwnState()
    {
        var sent = new Sent();
        var screens = Screens(sent);
        screens.Apply(State(on: true, allowed: true, allowedTo: "on_device"));
        Assert.True(screens.Polish.Consent.IsAllowedOn);
        Assert.False(screens.EditConsent.IsAllowedOn);
    }
}

public class MeetingsConsentTests
{
    private static SettingsHarness Screens(Sent sent)
    {
        var screens = new SettingsHarness(sent.Send);
        screens.Apply(Ev.Of(LocalLlm), State(on: false, allowed: false, feature: "meetings"));
        return screens;
    }

    /// <summary>
    /// Owner decision: summaries and Ask are off until the user turns them on in Settings > AI
    /// through the consent step, which names where the transcript goes; Cancel sends nothing;
    /// Allow asks the core to record it; off sends off (the core withdraws the consent with it).
    /// While off, a record without a summary says why.
    /// </summary>
    [Fact]
    public void TurningThemOnAsksFirstAndARecordSaysTheyAreOff()
    {
        var sent = new Sent();
        var screens = Screens(sent);
        var ai = screens.Ai;
        Assert.False(ai.MeetingsAIOn);
        Assert.True(ai.CanToggleMeetingsAI);
        Assert.Equal("Off. Meetings are recorded and transcribed, with no summary, and Ask stays off.", ai.MeetingsAIStatus);
        Assert.Equal("Summaries are off until you allow them in Settings > AI. Meetings are still recorded and transcribed.", ai.SummaryOffNote);

        var before = sent.Commands.Count;
        ai.SetMeetingsAI(true);
        var asked = Assert.IsType<ConsentDestination>(screens.MeetingsConsent.Pending);
        Assert.Equal(ConsentHost.Settings, screens.MeetingsConsent.Host);
        Assert.Equal(before, sent.Commands.Count); // asking sends nothing
        Assert.Equal("Turn on summaries and Ask?", ConsentModel.Title(LlmFeature.Meetings));
        var message = ConsentModel.Message(LlmFeature.Meetings, asked);
        Assert.Contains("send a meeting's transcript, what everyone in it said, to a language model", message, StringComparison.Ordinal);
        Assert.Contains("Example Local Model, on this PC, so the transcript stays on this PC", message, StringComparison.Ordinal);
        Assert.Equal("Turn On Summaries and Ask", ConsentModel.Button(LlmFeature.Meetings, asked));
        screens.MeetingsConsent.Cancel();
        Assert.Null(screens.MeetingsConsent.Pending);
        Assert.Equal(before, sent.Commands.Count); // cancel sends nothing
        Assert.False(ai.MeetingsAIOn); // cancel leaves them off

        ai.SetMeetingsAI(true);
        screens.MeetingsConsent.Allow();
        Assert.Equal(new CoreCommand.ConsentAllow(LlmFeature.Meetings, LlmDestination.OnDevice, null, null, "consent.allow:meetings:2"), sent.Commands[^1]);
        Assert.False(ai.MeetingsAIOn); // on only once the core has recorded it
        screens.Apply(State(on: true, allowed: true, allowedTo: "on_device", feature: "meetings"));
        Assert.True(ai.MeetingsAIOn);
        Assert.Null(ai.SummaryOffNote);
        Assert.Equal("On, with Example Local Model, on this PC. Meeting transcripts stay on this PC.", ai.MeetingsAIStatus);

        ai.SetMeetingsAI(false);
        Assert.Equal(new CoreCommand.SettingSet(ShellSetting.MeetingsLlm, "off"), sent.Commands[^1]);
        Assert.False(ai.MeetingsAIOn);
        Assert.NotNull(ai.SummaryOffNote);
    }

    /// <summary>A cloud model is named, and the step says the transcript leaves this PC for it.</summary>
    [Fact]
    public void ACloudModelIsNamedAndTheTranscriptLeavesThisMac()
    {
        var sent = new Sent();
        var screens = Screens(sent);
        screens.Apply(State(on: false, allowed: false, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", feature: "meetings"));
        screens.Ai.SetMeetingsAI(true);
        var asked = Assert.IsType<ConsentDestination>(screens.MeetingsConsent.Pending);
        var message = ConsentModel.Message(LlmFeature.Meetings, asked);
        Assert.Contains("the transcript leaves this PC and goes to Example Cloud", message, StringComparison.Ordinal);
        Assert.Equal("Send to Example Cloud", ConsentModel.Button(LlmFeature.Meetings, asked));
        Assert.Equal("Turn on summaries and Ask and send the transcript to Example Cloud", ConsentModel.AllowName(LlmFeature.Meetings, asked));
        screens.MeetingsConsent.Allow();
        Assert.Equal(new CoreCommand.ConsentAllow(LlmFeature.Meetings, LlmDestination.Cloud, "shell engine cloud", null, "consent.allow:meetings:2"), sent.Commands[^1]);
        screens.Apply(State(on: true, allowed: true, to: "cloud", name: "Example Cloud", endpoint: "shell engine cloud", allowedTo: "cloud", feature: "meetings"));
        Assert.Equal("On. Meeting transcripts go to Example Cloud for summaries and Ask.", screens.Ai.MeetingsAIStatus);
    }

    /// <summary>A meeting that ended without a summary for want of consent reads the state again; a switch-off the core refused is shown and put back.</summary>
    [Fact]
    public void ARefusedSummaryReadsTheStateAgainAndAFailedOffIsShown()
    {
        var sent = new Sent();
        var screens = Screens(sent);
        var before = sent.Commands.Count;
        screens.Apply(Ev.Of("""{"type":"meeting.warning","record":"r1","kind":"summary_not_allowed","message":"a model on this machine"}"""));
        Assert.Equal(new CoreCommand.ConsentGet(LlmFeature.Meetings, "consent.get:meetings:2"), sent.Commands[before]);

        screens.Apply(State(on: true, allowed: true, allowedTo: "on_device", feature: "meetings"));
        screens.Ai.SetMeetingsAI(false);
        Assert.False(screens.Ai.MeetingsAIOn);
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:meetings.llm","message":"x"}""");
        Assert.True(screens.Handles(failed)); // shown under the switch, not only logged
        screens.Apply(failed);
        Assert.True(screens.Ai.MeetingsAIOn); // put back
        Assert.Equal("Couldn't save the change, so summaries and Ask stay as they were.", screens.Ai.MeetingsAIStatus);
    }

    /// <summary>Each feature reads only its own state: summaries and Ask allowed allows neither polish nor voice edit, nor the reverse.</summary>
    [Fact]
    public void EachFeatureReadsOnlyItsOwnState()
    {
        var sent = new Sent();
        var screens = Screens(sent);
        screens.Apply(State(on: true, allowed: true, allowedTo: "on_device", feature: "meetings"));
        Assert.True(screens.Ai.MeetingsAIOn);
        Assert.False(screens.Polish.Consent.IsAllowedOn);
        Assert.False(screens.EditConsent.IsAllowedOn);
        screens.Apply(
            State(on: false, allowed: false, feature: "meetings"),
            State(on: true, allowed: true, allowedTo: "on_device"),
            State(on: true, allowed: true, allowedTo: "on_device", feature: "edit"));
        Assert.False(screens.Ai.MeetingsAIOn);
        Assert.NotNull(screens.Ai.SummaryOffNote);
    }

    /// <summary>Windows: with no language model the switch cannot be used, and says why in this PC's words.</summary>
    [Fact]
    public void WithNoModelTheSwitchSaysSoAndSendsNothing()
    {
        var sent = new Sent();
        var screens = new SettingsHarness(sent.Send);
        screens.Apply(State(on: false, allowed: false, feature: "meetings"));
        Assert.False(screens.Ai.CanToggleMeetingsAI);
        Assert.Equal("No language model is available on this PC, so meetings get no summary.", screens.Ai.MeetingsAIStatus);
        screens.Ai.SetMeetingsAI(true);
        Assert.Null(screens.MeetingsConsent.Pending);
        Assert.Empty(sent.Commands);
    }

    /// <summary>A view bound to AiSettings hears about a change in any model it reads.</summary>
    [Fact]
    public void AChangeInAConsentIsAChangeOfTheSettings()
    {
        var screens = new SettingsHarness(_ => { });
        var before = screens.Ai.Changes;
        screens.Apply(State(on: true, allowed: true, feature: "meetings"));
        Assert.True(screens.Ai.Changes > before);
    }
}
