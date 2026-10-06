// Settings > AI's "On this PC": Use of this PC's own model, then polish's one tap, as the core
// answers. The model's id and name are the core's row (public); nothing else here is real.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.SettingsEvents;

namespace Inkwell.Core.Tests.Screens;

public class LocalLlmModelTests
{
    private const string Id = "qwen3-4b-instruct-2507-q4km";

    private static InkEvent Providers(string? chosen, bool ready, string? reference = null)
    {
        var choice = chosen is null ? "" : $",\"chosen\":\"{chosen}\",\"model\":\"{Id}\",\"endpoint\":\"this process\",\"to\":\"on_device\"";
        var answer = reference is null ? "" : $",\"ref\":\"{reference}\"";
        return Ev.Of(
            "{\"type\":\"llm.providers\",\"providers\":["
            + "{\"id\":\"groq\",\"default_model\":\"llama-3.3-70b-versatile\",\"endpoint\":\"https://api.groq.com/openai/v1\",\"custom_url\":false,\"needs_key\":true,\"has_key\":true},"
            + $"{{\"id\":\"on_device\",\"default_model\":\"{Id}\",\"endpoint\":\"this process\",\"custom_url\":false,\"needs_key\":false,\"has_key\":false,\"installed\":true}}]"
            + $"{choice},\"local_only\":true,\"ready\":{(ready ? "true" : "false")}{answer}}}");
    }

    /// <summary>
    /// Use chooses this PC's model first (local-only mode stays on, no model or address named),
    /// and only once the core has chosen it does polish's one tap show: on this PC, "Turn On
    /// Polish", your words staying here. Its tap allows polish for this PC alone, with no endpoint
    /// or key; nothing asks before the choice is answered.
    /// </summary>
    [Fact]
    public void UseChoosesThisPcsModelThenPolishAsksOnce()
    {
        var sent = new Sent();
        var screens = new ScreenModels(sent.Send);
        screens.Apply([
            Ev.Of($$"""{"type":"models.listed","models":[{"id":"{{Id}}","kind":"language","name":"Qwen3 4B Instruct","licence":"Apache-2.0","size_bytes":2497281120,"installed":true,"jobs":[]}],"free_bytes":50000000000}"""),
            Providers(chosen: null, ready: false),
            State(on: false, allowed: false, to: "on_device", name: "Qwen3 4B Instruct"),
        ]);
        Assert.True(screens.Polish.HasWorkingEngine); // downloaded with none chosen: the core uses it
        Assert.True(screens.Local.InUse);

        screens.Cloud.Select(CloudModel.OnDeviceId);
        Assert.Equal("Use this PC's model", screens.Cloud.UseLabel);
        Assert.True(screens.Cloud.Use());
        var choose = Assert.IsType<CoreCommand.LlmChoose>(sent.Commands[^1]);
        Assert.Equal(new CoreCommand.LlmChoose("on_device", null, null, false, choose.Ref), choose);
        Assert.Null(screens.Polish.Consent.Pending); // nothing asks before the core has chosen it

        // The core's answer to the choice: local-only mode, each feature's state, then the providers with the ref.
        screens.Apply([
            Ev.Of("""{"type":"setting.value","key":"llm.local_only","value":"on"}"""),
            State(on: false, allowed: false, to: "on_device", name: "Qwen3 4B Instruct"),
            Providers(chosen: "on_device", ready: true, reference: choose.Ref),
        ]);
        var step = screens.Polish.Consent.Pending;
        Assert.NotNull(step);
        Assert.True(step.IsOnDevice);
        Assert.True(screens.Polish.Consent.IsShowingStep(ConsentHost.Settings));
        Assert.Equal("Turn On Polish", PolishModel.ConsentButton(step));
        Assert.EndsWith("It uses Qwen3 4B Instruct, on this PC, so your words stay on this PC.", PolishModel.ConsentMessage(step));
        Assert.False(ConsentModel.FocusesCancel(step)); // one tap: the agreeing button has the focus

        var before = sent.Commands.Count;
        screens.Polish.AllowConsent();
        var allow = Assert.IsType<CoreCommand.ConsentAllow>(Assert.Single(sent.Commands.Skip(before)));
        Assert.Equal(LlmFeature.Polish, allow.Feature);
        Assert.Equal(LlmDestination.OnDevice, allow.To);
        Assert.Null(allow.Endpoint);
        Assert.Null(allow.Key);
        Assert.Null(screens.Polish.Consent.Pending);
    }
}
