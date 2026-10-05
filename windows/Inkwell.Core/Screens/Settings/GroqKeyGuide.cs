// How to get a free Groq key, step by step: the first run's, over Groq's key box in the Polish
// step, and Settings > AI's, under the language model's rows. The Mac's GroqKeyGuide, in the same
// words (neither names its platform).
//
// Only what Groq's own pages say, checked 2026-10-05:
// - https://console.groq.com/keys ("API Keys - GroqCloud"): the "Create API Key" button, a Name for
//   each key, and "Remember to keep your API keys safe".
// - https://console.groq.com/settings/billing/plans: "Free", "Great for anyone to get started with
//   our APIs", "$0".
// - https://console.groq.com/docs/rate-limits: the "Free Plan Limits" table.
// None of them says whether the Free plan needs a card or whether a key is shown only once, so the
// guide says neither.
namespace Inkwell.Core.Screens;

/// <summary>Where the guide is, which is what its last step says to press.</summary>
public enum GroqKeyGuidePlace
{
    /// <summary>The Polish step, over Groq's key box, its Save and Use Groq.</summary>
    FirstRun,
    /// <summary>Settings > AI, under the language model's picker, Save key and Use Groq.</summary>
    Settings,
}

public static class GroqKeyGuide
{
    /// <summary>Settings' disclosure over the guide.</summary>
    public const string Title = "How to get a free Groq key";

    public const string Cost = "Groq's Free plan costs $0 and has rate limits, listed on its Rate Limits page.";

    /// <summary>The part of <see cref="Cost"/> that is the link to <see cref="RateLimitsUrl"/>.</summary>
    public const string RateLimitsLink = "Rate Limits page";

    public const string RateLimitsUrl = "https://console.groq.com/docs/rate-limits";

    public const string KeysUrl = "https://console.groq.com/keys";

    /// <summary>The first step's link, as it reads.</summary>
    public const string KeysLink = "console.groq.com/keys";

    /// <summary>The first step's link, as Narrator names it.</summary>
    public const string KeysLinkName = "Open Groq's API Keys page in your browser";

    /// <summary>
    /// The Mac's four steps (copying the key and pasting it are one there, so its fixed sheet
    /// holds them). The first is followed by its link.
    /// </summary>
    public static IReadOnlyList<string> Steps(GroqKeyGuidePlace place) =>
    [
        "Open Groq's API Keys page:",
        "Log in, or make a Groq account.",
        "Press Create API Key and give the key a name, such as Inkwell.",
        place == GroqKeyGuidePlace.FirstRun
            ? "Copy the key, paste it below and press Save, then Use Groq."
            : "Copy the key, choose Groq above, paste it and press Save key, then Use Groq.",
    ];

    /// <summary>A step as Narrator reads it: its number and how many there are, then the words.</summary>
    public static string StepName(int number, int count, string text) => $"Step {number} of {count}: {text}";
}
