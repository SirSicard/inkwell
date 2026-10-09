// Settings > AI's list of where polish may send (one consent per destination), and Revoke. As the
// Mac's PolishConsentsTests.
using System.Text.Json.Nodes;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class PolishConsentsTests
{
    private const string GroqConsent = """{"to":"cloud","name":"Groq","endpoint":"https://api.groq.com/openai/v1"}""";
    private const string DeviceConsent = """{"to":"on_device"}""";

    private static InkEvent State(bool on, IEnumerable<string> consents, string? reference = null) =>
        Ev.Of($$"""{"type":"consent.state","feature":"polish","on":{{(on ? "true" : "false")}},"allowed":true,"to":"on_device","name":"Example Local Model","consents":[{{string.Join(",", consents)}}]{{(reference is null ? "" : $",\"ref\":\"{reference}\"")}}}""");

    private static InkEvent Failed(string command, string id) =>
        Ev.Of("{\"type\":\"command.failed\",\"command\":\"" + command + "\",\"id\":\"" + id + "\",\"message\":\"refused\"}");

    [Fact]
    public void EachDestinationIsListedAndRevokedOnItsOwn()
    {
        var sent = new Sent();
        var consent = new ConsentModel(LlmFeature.Polish, PolishModel.SettingId, sent.Send);
        consent.Apply(State(true, [DeviceConsent, GroqConsent, """{"to":"cloud"}"""]));
        var consents = consent.State!.Consents;
        Assert.Equal(["Models on this PC", "Groq"], consents.Select(c => c.Label)); // a cloud OK without its endpoint can't be named or revoked
        Assert.Equal(["Your words stay on this PC.", "Your words leave this PC for api.groq.com."], consents.Select(c => c.Detail));
        Assert.True(consent.State.Covers(ConsentDestination.Cloud("https://api.groq.com/openai/v1/", "")));
        Assert.False(consent.State.Covers(ConsentDestination.Cloud("https://api.openai.com/v1", "")));

        consent.Revoke(consents[1]);
        Assert.Equal([new CoreCommand.ConsentRevoke(LlmFeature.Polish, LlmDestination.Cloud, "https://api.groq.com/openai/v1", "consent.revoke:polish:1")], sent.Commands);
        var json = JsonNode.Parse(sent.Commands[0].Json)!.AsObject();
        Assert.Equal(["cmd", "endpoint", "feature", "id", "to"], json.Select(f => f.Key).Order());
        Assert.Equal("consent.revoke", json["cmd"]!.GetValue<string>());
        consent.Revoke(consents[0]);
        Assert.Equal(new CoreCommand.ConsentRevoke(LlmFeature.Polish, LlmDestination.OnDevice, null, "consent.revoke:polish:2"), sent.Commands[^1]);
        var failed = Failed("consent.revoke", "consent.revoke:polish:2");
        Assert.True(consent.Handles((CommandFailed)failed));
        consent.Apply(failed);
        Assert.Equal("Couldn't revoke that, so polish may still send there. Try again.", consent.Problem);
        consent.Apply(State(false, [], "consent.revoke:polish:2"));
        Assert.Null(consent.Failure);
        Assert.Empty(consent.State!.Consents);
    }

    /// <summary>A mode's OK is matched by its asker, never shown as the switch's failure.</summary>
    [Fact]
    public void AModesOkFailingIsNotTheSwitchsFailure()
    {
        var consent = new ConsentModel(LlmFeature.Polish, PolishModel.SettingId, _ => { });
        var reference = consent.AllowForMode(ConsentDestination.OnDevice("x"));
        consent.Apply(Failed("consent.allow", reference));
        Assert.Null(consent.Failure);
    }
}
