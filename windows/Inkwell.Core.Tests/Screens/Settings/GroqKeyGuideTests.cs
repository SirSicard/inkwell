// How to get a free Groq key (the Mac's GroqKeyGuideTests): the first run's and Settings > AI's
// steps, the pages they open, the names Narrator reads, and the Mac's words, the same.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class GroqKeyGuideTests
{
    private static readonly string[] Shared =
    [
        "Open Groq's API Keys page:",
        "Log in, or make a Groq account.",
        "Press Create API Key and give the key a name, such as Inkwell.",
    ];

    [Fact]
    public void TheStepsSayWhatToDoInOrderAndEndWhereTheGuideIs()
    {
        Assert.Equal("How to get a free Groq key", GroqKeyGuide.Title);
        Assert.Equal("Groq's Free plan costs $0 and has rate limits, listed on its Rate Limits page.", GroqKeyGuide.Cost);
        Assert.Contains(GroqKeyGuide.RateLimitsLink, GroqKeyGuide.Cost, StringComparison.Ordinal);
        Assert.Equal(
            [.. Shared, "Copy the key, paste it below and press Save, then Use Groq."],
            GroqKeyGuide.Steps(GroqKeyGuidePlace.FirstRun));
        Assert.Equal(
            [.. Shared, "Copy the key, choose Groq above, paste it and press Save key, then Use Groq."],
            GroqKeyGuide.Steps(GroqKeyGuidePlace.Settings));
    }

    /// <summary>The buttons the last steps name are the ones beside them: Save, Save key, Use Groq.</summary>
    [Fact]
    public void TheLastStepsNameTheButtonsBesideThem()
    {
        Assert.Contains($"press {OnboardingModel.OwnKeySave},", GroqKeyGuide.Steps(GroqKeyGuidePlace.FirstRun)[^1], StringComparison.Ordinal);
        var cloud = new CloudModel(new Sent().Send);
        cloud.Load();
        cloud.Apply(CloudModelTests.Providers());
        cloud.Select(OnboardingModel.OwnKeyProvider);
        Assert.EndsWith($"then {cloud.UseLabel}.", GroqKeyGuide.Steps(GroqKeyGuidePlace.FirstRun)[^1], StringComparison.Ordinal);
        Assert.EndsWith($"then {cloud.UseLabel}.", GroqKeyGuide.Steps(GroqKeyGuidePlace.Settings)[^1], StringComparison.Ordinal);
        // Settings' LanguageModelRows.xaml labels its button "Save key".
        Assert.Contains("Content=\"Save key\"", AboutCheckout.ReadWindows("Inkwell/Screens/Settings/LanguageModelRows.xaml"), StringComparison.Ordinal);
    }

    [Fact]
    public void TheLinksOpenGroqsOwnPagesAndAreNamedForNarrator()
    {
        Assert.Equal("https://console.groq.com/keys", GroqKeyGuide.KeysUrl);
        Assert.Equal("https://" + GroqKeyGuide.KeysLink, GroqKeyGuide.KeysUrl);
        Assert.Equal("https://console.groq.com/docs/rate-limits", GroqKeyGuide.RateLimitsUrl);
        Assert.Equal("Open Groq's API Keys page in your browser", GroqKeyGuide.KeysLinkName);
        Assert.Equal("Step 2 of 4: Log in, or make a Groq account.", GroqKeyGuide.StepName(2, 4, "Log in, or make a Groq account."));
    }

    /// <summary>The Mac's guide (SettingsGlowSections.swift) has every one of these strings, word for word.</summary>
    [Fact]
    public void TheWordsAreTheMacs()
    {
        var mac = AboutCheckout.Read("mac/Sources/Inkwell/Screens/SettingsGlowSections.swift");
        string[] all =
        [
            GroqKeyGuide.Title, GroqKeyGuide.Cost, GroqKeyGuide.RateLimitsLink, GroqKeyGuide.RateLimitsUrl,
            GroqKeyGuide.KeysUrl, GroqKeyGuide.KeysLink, GroqKeyGuide.KeysLinkName,
            .. GroqKeyGuide.Steps(GroqKeyGuidePlace.FirstRun), .. GroqKeyGuide.Steps(GroqKeyGuidePlace.Settings),
        ];
        Assert.All(all, text => Assert.Contains($"\"{text}\"", mac, StringComparison.Ordinal));
    }
}
