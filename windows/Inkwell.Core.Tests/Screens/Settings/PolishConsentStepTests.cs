// Polish's consent step in the first-run sheet: the Mac's modal alert, drawn inside the sheet
// because the sheet is a ContentDialog and WinUI shows one at a time. Where it goes, what waits
// while it is up, what Narrator hears when it appears, and where focus lands.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.SettingsEvents;

namespace Inkwell.Core.Tests.Screens;

public class PolishConsentStepTests
{
    /// <summary>
    /// Skip, Back and Continue wait while the sheet's own step is up, as nothing moves behind the
    /// Mac's alert; Cancel or Allow answers it first. A step Settings asked for (its own dialog,
    /// under the sheet) never holds the sheet.
    /// </summary>
    [Fact]
    public void TheSheetsButtonsWaitWhileItsStepIsUp()
    {
        var sent = new Sent();
        var cloud = new CloudModel(sent.Send);
        var polish = new PolishModel(sent.Send);
        cloud.Apply(CloudModelTests.Providers(keyed: ["groq"]));
        cloud.Suggest("groq");
        Assert.True(OnboardingModel.CanNavigate(polish.Consent, OnboardingStep.Polish));

        polish.UseOwnKey(cloud);
        Assert.True(polish.Consent.IsShowingStep(ConsentHost.Onboarding));
        Assert.False(OnboardingModel.CanNavigate(polish.Consent, OnboardingStep.Polish));
        polish.CancelConsent();
        Assert.True(OnboardingModel.CanNavigate(polish.Consent, OnboardingStep.Polish));

        polish.UseOwnKey(cloud);
        Assert.False(OnboardingModel.CanNavigate(polish.Consent, OnboardingStep.Polish));
        polish.AllowConsent();
        Assert.False(polish.Consent.IsShowingStep(ConsentHost.Onboarding));
        Assert.True(OnboardingModel.CanNavigate(polish.Consent, OnboardingStep.Polish));

        // Up but not on screen (the model moved the sheet off Polish): the buttons work, and Escape
        // still cancels the step.
        polish.UseOwnKey(cloud);
        Assert.False(OnboardingModel.CanNavigate(polish.Consent, OnboardingStep.Polish));
        Assert.True(OnboardingModel.CanNavigate(polish.Consent, OnboardingStep.Ready));
        polish.CancelConsent();

        polish.Apply(Ev.Of(LocalLlm));
        polish.Apply(State(on: false, allowed: false));
        polish.SetOn(true, ConsentHost.Settings);
        Assert.True(polish.Consent.IsShowingStep(ConsentHost.Settings));
        Assert.True(OnboardingModel.CanNavigate(polish.Consent, OnboardingStep.Polish));
    }

    /// <summary>
    /// The step shows under what asked for it: the switch, Use Groq, or the other providers' Use.
    /// With the own key's disclosure closed, a Use step would be hidden inside it, so it goes under
    /// the switch.
    /// </summary>
    [Theory]
    [InlineData(true, true, false, PolishStepPlace.UnderSwitch)]
    [InlineData(true, false, false, PolishStepPlace.UnderSwitch)]
    [InlineData(false, true, false, PolishStepPlace.UnderGroqUse)]
    [InlineData(false, true, true, PolishStepPlace.UnderOthersUse)]
    [InlineData(false, false, false, PolishStepPlace.UnderSwitch)]
    [InlineData(false, false, true, PolishStepPlace.UnderSwitch)]
    public void TheStepShowsUnderWhatAskedForIt(bool askedBySwitch, bool ownKeyOpen, bool others, PolishStepPlace place) =>
        Assert.Equal(place, OnboardingModel.PolishStepPlaceFor(askedBySwitch, ownKeyOpen, others));

    /// <summary>
    /// The place is settled when the step is asked for, and kept while its rows show: the step
    /// never moves under a Use that did not ask (the other providers' rows opened meanwhile). When
    /// its rows are hidden (the disclosure closed, or the other rows shown instead) it shows under
    /// the switch, so it is never hidden.
    /// </summary>
    [Theory]
    [InlineData(PolishStepPlace.UnderGroqUse, true, false, PolishStepPlace.UnderGroqUse)]
    [InlineData(PolishStepPlace.UnderGroqUse, true, true, PolishStepPlace.UnderSwitch)]
    [InlineData(PolishStepPlace.UnderGroqUse, false, false, PolishStepPlace.UnderSwitch)]
    [InlineData(PolishStepPlace.UnderOthersUse, true, true, PolishStepPlace.UnderOthersUse)]
    [InlineData(PolishStepPlace.UnderOthersUse, true, false, PolishStepPlace.UnderSwitch)]
    [InlineData(PolishStepPlace.UnderOthersUse, false, true, PolishStepPlace.UnderSwitch)]
    [InlineData(PolishStepPlace.UnderSwitch, true, false, PolishStepPlace.UnderSwitch)]
    public void TheStepStaysWhereItWasAskedWhileThoseRowsShow(PolishStepPlace asked, bool ownKeyOpen, bool others, PolishStepPlace shown) =>
        Assert.Equal(shown, OnboardingModel.PolishStepShownAt(asked, ownKeyOpen, others));

    /// <summary>Narrator hears the step's heading and where the words would go as it appears.</summary>
    [Fact]
    public void NarratorHearsTheHeadingAndTheDestination()
    {
        var groq = ConsentDestination.Cloud("https://api.groq.com/openai/v1", "Groq");
        var announced = ConsentModel.Announcement(LlmFeature.Polish, groq);
        Assert.Equal($"Turn on polish? {ConsentModel.Message(LlmFeature.Polish, groq)}", announced);
        Assert.StartsWith(ConsentModel.Title(LlmFeature.Polish), announced, StringComparison.Ordinal);
        Assert.Contains("go to Groq.", announced, StringComparison.Ordinal);
    }

    /// <summary>
    /// Focus goes to Cancel for a provider off this PC, so Enter never agrees to send words away
    /// (Settings' dialog keeps the same rule), and to the agreeing button for one on this PC.
    /// </summary>
    [Fact]
    public void FocusLandsOnCancelForACloudProvider()
    {
        Assert.True(ConsentModel.FocusesCancel(ConsentDestination.Cloud("https://api.groq.com/openai/v1", "Groq")));
        Assert.False(ConsentModel.FocusesCancel(ConsentDestination.OnDevice("Local server")));
    }
}
