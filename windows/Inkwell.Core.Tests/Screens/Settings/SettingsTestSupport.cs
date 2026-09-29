// What the Settings tests share: consent.state and engine events as the core sends them, and the
// Settings models wired as the Mac's ScreenModels wires them (apply order, the loads at core.ready,
// handles), so the Mac tests that went through ScreenModels port with the same intent. The real
// wiring is the aggregator's; this mirrors ScreenModels.swift for these models only.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;

namespace Inkwell.Core.Tests.Screens;

internal static class SettingsEvents
{
    /// <summary>A language model the core registered (a synthetic id).</summary>
    public const string LocalLlm = """{"type":"engine.registered","id":"local-llm","kind":"llm","jobs":[]}""";

    /// <summary>The on-device model's name in these tests (synthetic).</summary>
    public const string LocalName = "Example Local Model";

    /// <summary>consent.state for polish (or <paramref name="feature"/>) as the core sends it.</summary>
    public static InkEvent State(
        bool on, bool allowed, string? to = "on_device", string name = LocalName, string? endpoint = null,
        string? allowedTo = null, string? error = null, string feature = "polish", string? reference = null)
    {
        var fields = new List<string> { "\"type\":\"consent.state\"", $"\"feature\":\"{feature}\"", $"\"on\":{Bool(on)}", $"\"allowed\":{Bool(allowed)}" };
        if (to is not null)
        {
            fields.Add($"\"to\":\"{to}\"");
            fields.Add($"\"name\":\"{name}\"");
        }
        void Add(string key, string? value)
        {
            if (value is not null)
            {
                fields.Add($"\"{key}\":\"{value}\"");
            }
        }
        Add("endpoint", endpoint);
        Add("allowed_to", allowedTo);
        Add("error", error);
        Add("ref", reference);
        return Ev.Of("{" + string.Join(",", fields) + "}");
    }

    private static string Bool(bool b) => b ? "true" : "false";
}

/// <summary>The Settings models, wired as ScreenModels wires them on the Mac.</summary>
internal sealed class SettingsHarness
{
    public SettingsHarness(Action<CoreCommand> send, ICalendarAccess? calendar = null, ScreenLog? log = null)
    {
        Permissions = new PermissionsModel(send, calendar);
        Polish = new PolishModel(send);
        Onboarding = new OnboardingModel(send, log);
        Dictation = new DictationModel(send);
        EditConsent = AiSettings.NewEditConsent(send);
        MeetingsConsent = AiSettings.NewMeetingsConsent(send);
        Ai = new AiSettings(Polish, Dictation, EditConsent, MeetingsConsent, send);
    }

    public PermissionsModel Permissions { get; }
    public PolishModel Polish { get; }
    public OnboardingModel Onboarding { get; }
    public DictationModel Dictation { get; }
    public ConsentModel EditConsent { get; }
    public ConsentModel MeetingsConsent { get; }
    public AiSettings Ai { get; }

    public void Apply(params InkEvent[] batch)
    {
        foreach (var e in batch)
        {
            if (e is CoreReady)
            {
                CoreReady();
            }
            Permissions.Apply(e);
            Polish.Apply(e);
            Onboarding.Apply(e);
            Dictation.Apply(e);
            EditConsent.Apply(e);
            MeetingsConsent.Apply(e);
        }
    }

    /// <summary>ScreenModels.coreReady, for these models.</summary>
    private void CoreReady()
    {
        Onboarding.Load();
        Polish.Load();
        Permissions.Refresh();
        Dictation.Load();
        EditConsent.Load();
        MeetingsConsent.Load();
    }

    /// <summary>ScreenModels.handles, for these models.</summary>
    public bool Handles(CommandFailed failed) =>
        PermissionsModel.Handles(failed) || Polish.Handles(failed) || OnboardingModel.Handles(failed)
        || DictationModel.Handles(failed) || Ai.Handles(failed);
}
