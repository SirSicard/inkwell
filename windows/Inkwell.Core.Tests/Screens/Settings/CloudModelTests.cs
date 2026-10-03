// Settings > AI's language model (Windows): an own-key provider picked, keyed, chosen and tested,
// as the core answers (llm.providers, llm.tested). Every key, model and address here is synthetic.
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.SettingsEvents;

namespace Inkwell.Core.Tests.Screens;

public class CloudModelTests
{
    private const string Key = "sk-synthetic-canary-9e2b";

    /// <summary>llm.providers as the core sends it: every provider, keys as given, and the choice.</summary>
    internal static InkEvent Providers(
        string? chosen = null, string? model = null, string? to = null, bool localOnly = true, bool ready = false,
        string[]? keyed = null, string? baseUrl = null, string? error = null, string? reference = null)
    {
        keyed ??= [];
        var list = new (string Id, string Model, string Endpoint)[]
        {
            ("openai", "gpt-4o-mini", "https://api.openai.com/v1"),
            ("groq", "llama-3.3-70b-versatile", "https://api.groq.com/openai/v1"),
            ("anthropic", "claude-haiku-4-5-20251001", "https://api.anthropic.com"),
            ("openrouter", "openai/gpt-4o-mini", "https://openrouter.ai/api/v1"),
            ("custom", "llama3", "http://localhost:11434/v1"),
        }.Select(p => new Dictionary<string, object>
        {
            ["id"] = p.Id,
            ["default_model"] = p.Model,
            ["endpoint"] = p.Endpoint,
            ["custom_url"] = p.Id == "custom",
            ["needs_key"] = p.Id != "custom",
            ["has_key"] = keyed.Contains(p.Id),
        }).ToList();
        var fields = new Dictionary<string, object> { ["type"] = "llm.providers", ["providers"] = list, ["local_only"] = localOnly, ["ready"] = ready };
        void Add(string key, string? value)
        {
            if (value is not null)
            {
                fields[key] = value;
            }
        }
        Add("chosen", chosen);
        Add("model", model);
        Add("to", to);
        Add("base_url", baseUrl);
        Add("endpoint", chosen is null ? null : chosen == "custom" ? baseUrl : list.First(p => (string)p["id"] == chosen)["endpoint"] as string);
        Add("error", error);
        Add("ref", reference);
        return Ev.Of(JsonSerializer.Serialize(fields));
    }

    private static (CloudModel Cloud, Sent Sent) Loaded(InkEvent? state = null)
    {
        var sent = new Sent();
        var cloud = new CloudModel(sent.Send);
        cloud.Load();
        cloud.Apply(state ?? Providers());
        return (cloud, sent);
    }

    [Fact]
    public void TheProvidersAreListedAndNothingIsChosenAtFirst()
    {
        var sent = new Sent();
        var cloud = new CloudModel(sent.Send);
        Assert.False(cloud.Loaded);
        Assert.False(cloud.CanUse);
        cloud.Load();
        Assert.Equal(new CoreCommand.LlmProviders("llm.providers:1"), sent.Commands[^1]);
        cloud.Apply(Providers());
        Assert.Equal(["OpenAI", "Groq", "Anthropic", "OpenRouter", "OpenAI-compatible server"], cloud.Providers.Select(p => p.Name));
        Assert.Null(cloud.Selected);
        Assert.True(cloud.LocalOnly);
        Assert.Equal("No language model is in use. Local-only mode is on.", cloud.Status);
        Assert.False(cloud.CanUse); // none is already none
        Assert.False(cloud.CanTest);
    }

    /// <summary>
    /// Picking a provider sends nothing; the key goes once, in llm.key.save, and is not kept; Use
    /// names the provider and carries the user's say-so that local-only mode goes off, which the
    /// step says first.
    /// </summary>
    [Fact]
    public void AKeyIsSentOnceAndACloudProviderIsUsedOnlyWithLocalOnlyOff()
    {
        var (cloud, sent) = Loaded();
        var before = sent.Commands.Count;
        cloud.Select("openai");
        Assert.Equal(before, sent.Commands.Count);
        Assert.Equal("No key is stored yet.", cloud.KeyStatus);
        Assert.True(cloud.SelectedIsCloud);
        Assert.Equal(
            "Using OpenAI turns local-only mode off, so the features below can send to OpenAI. Each one sends only once you allow it for OpenAI; one you already allowed for OpenAI sends again straight away.",
            cloud.UseNote);

        cloud.SaveKey("   ");
        Assert.Equal(before, sent.Commands.Count); // nothing to send
        Assert.Equal("Type or paste the key first.", cloud.Failure);

        cloud.SaveKey(Key);
        var save = Assert.IsType<CoreCommand.LlmKeySave>(sent.Commands[^1]);
        Assert.Equal("openai", save.Provider);
        Assert.Null(cloud.Failure);
        // The key travels in the JSON the core reads, and nowhere else: not in the model, not in the command's text.
        Assert.Contains(Key, save.Json, StringComparison.Ordinal);
        Assert.DoesNotContain(Key, save.ToString(), StringComparison.Ordinal);
        Assert.DoesNotContain(Key, save.Name, StringComparison.Ordinal);
        cloud.Apply(Providers(keyed: ["openai"], reference: save.Ref));
        Assert.Equal("A key is stored for this Windows account, in Windows Credential Manager: every Inkwell on this account uses it, whichever library it opens.", cloud.KeyStatus);
        Assert.Equal("openai", cloud.Selected); // saving a key keeps the picker where it was

        cloud.DraftModel = " gpt-synthetic ";
        Assert.Equal("Use OpenAI", cloud.UseLabel);
        cloud.Use();
        Assert.Equal(new CoreCommand.LlmChoose("openai", "gpt-synthetic", null, true, "llm.choose:3"), sent.Commands[^1]);
        var json = JsonDocument.Parse(sent.Commands[^1].Json).RootElement;
        Assert.Equal("off", json.GetProperty("local_only").GetString());
        Assert.False(json.TryGetProperty("base_url", out _));

        cloud.Apply(Providers("openai", "gpt-synthetic", "cloud", localOnly: false, ready: true, keyed: ["openai"], reference: "llm.choose:3"));
        Assert.Equal("In use: gpt-synthetic at OpenAI. Local-only mode is off.", cloud.Status);
        Assert.False(cloud.CanUse); // nothing to change
        Assert.True(cloud.CanTest);
    }

    /// <summary>
    /// The first run's own key (onboarding's Polish step): one press picks the provider and stores
    /// the key once; only once the core has stored it is the provider chosen, with its default
    /// model (the commands Settings > AI's picker, Save key and Use send). A key the core refuses
    /// chooses nothing, so local-only mode stays on, and the refusal stays shown. No key, or
    /// providers not read yet, sends nothing and says so.
    /// </summary>
    [Fact]
    public void TheFirstRunsOwnKeyIsChosenOnlyOnceTheCoreHasStoredIt()
    {
        var early = new Sent();
        var unread = new CloudModel(early.Send);
        unread.UseKey("groq", Key);
        Assert.Empty(early.Commands);
        Assert.NotNull(unread.Failure);

        var (cloud, sent) = Loaded();
        var before = sent.Commands.Count;
        cloud.UseKey("groq", "  ");
        Assert.Equal(before, sent.Commands.Count);
        Assert.Equal("Type or paste the key first.", cloud.Failure);

        // Refused: nothing is chosen, and the refusal survives the providers read after it.
        cloud.UseKey("groq", Key);
        var refused = Assert.IsType<CoreCommand.LlmKeySave>(sent.Commands[^1]);
        Assert.Equal(before + 1, sent.Commands.Count);
        cloud.Apply(Ev.Of($$"""{"type":"command.failed","command":"llm.key.save","id":"{{refused.Ref}}","message":"couldn't store the key"}"""));
        cloud.Apply(Providers());
        Assert.Equal(before + 1, sent.Commands.Count);
        Assert.Equal("Couldn't store the key.", cloud.Failure);

        // Stored: then chosen.
        cloud.UseKey("groq", Key);
        var save = Assert.IsType<CoreCommand.LlmKeySave>(sent.Commands[^1]);
        Assert.Equal("groq", save.Provider);
        Assert.Null(cloud.Failure);
        cloud.Apply(Providers(keyed: ["groq"], reference: save.Ref));
        var choose = Assert.IsType<CoreCommand.LlmChoose>(sent.Commands[^1]);
        Assert.Equal("groq", choose.Provider);
        Assert.Null(choose.Model); // the provider's default
        Assert.True(choose.LocalOnlyOff); // the step says so before the press
        Assert.Equal("groq", cloud.Selected);
        // Chosen once: the providers read after the choice sends nothing more.
        var after = sent.Commands.Count;
        cloud.Apply(Providers("groq", "llama-3.3-70b-versatile", "cloud", localOnly: false, ready: true, keyed: ["groq"], reference: choose.Ref));
        Assert.Equal(after, sent.Commands.Count);
        Assert.True(cloud.Ready);
    }

    /// <summary>A server on this PC keeps local-only mode on: Use says so, and sends no say-so.</summary>
    [Fact]
    public void AServerOnThisPcKeepsLocalOnlyOn()
    {
        var (cloud, sent) = Loaded();
        cloud.Select("custom");
        Assert.Equal("http://localhost:11434/v1", cloud.DraftBaseUrl);
        Assert.False(cloud.SelectedIsCloud);
        Assert.Equal("No key is needed unless your server asks for one.", cloud.KeyStatus);
        Assert.StartsWith("This server is on this PC, so local-only mode stays on.", cloud.UseNote, StringComparison.Ordinal);
        cloud.DraftBaseUrl = "http://127.0.0.1:8080/v1";
        cloud.Use();
        Assert.Equal(new CoreCommand.LlmChoose("custom", null, "http://127.0.0.1:8080/v1", false, "llm.choose:2"), sent.Commands[^1]);
        Assert.False(JsonDocument.Parse(sent.Commands[^1].Json).RootElement.TryGetProperty("local_only", out _));

        // The same provider on another machine is a cloud provider like any other.
        cloud.DraftBaseUrl = "https://llm.example.com/v1";
        Assert.True(cloud.SelectedIsCloud);
        cloud.Use();
        Assert.True(Assert.IsType<CoreCommand.LlmChoose>(sent.Commands[^1]).LocalOnlyOff);
    }

    /// <summary>
    /// A custom server over plain http off this PC never gets the key (the core keeps it back), and
    /// the key line says so, stored or not, instead of reading as if the key were used.
    /// </summary>
    [Fact]
    public void AKeyKeptBackFromAPlainHttpServerIsSaid()
    {
        var (cloud, _) = Loaded(Providers(keyed: ["custom"]));
        cloud.Select("custom");
        Assert.False(cloud.KeyWithheld); // Ollama's default, on this PC
        Assert.Equal("A key is stored for this Windows account, in Windows Credential Manager: every Inkwell on this account uses it, whichever library it opens.", cloud.KeyStatus);
        cloud.DraftBaseUrl = "http://192.0.2.10:8000/v1";
        Assert.True(cloud.KeyWithheld);
        Assert.Equal(
            "A key is stored for this Windows account, in Windows Credential Manager, but it is not sent to this server: keys go only over https or to a server on this PC.",
            cloud.KeyStatus);
        cloud.DraftBaseUrl = "https://llm.example.com/v1";
        Assert.False(cloud.KeyWithheld);
        Assert.Equal("A key is stored for this Windows account, in Windows Credential Manager: every Inkwell on this account uses it, whichever library it opens.", cloud.KeyStatus);

        (cloud, _) = Loaded();
        cloud.Select("custom");
        cloud.DraftBaseUrl = "HTTP://llm.example.com/v1";
        Assert.Equal("No key is sent to this server: keys go only over https or to a server on this PC.", cloud.KeyStatus);
        cloud.Select("openai");
        Assert.False(cloud.KeyWithheld); // a built-in provider's address is https
    }

    [Theory]
    [InlineData("http://localhost:11434/v1", true)]
    [InlineData("http://LOCALHOST/v1", true)]
    [InlineData("http://127.0.0.1:8080/v1", true)]
    [InlineData("https://127.255.255.254/", true)]
    [InlineData("http://[::1]:8080/v1", true)]
    [InlineData("http://localhost.example.com/v1", false)]
    [InlineData("http://127.0.0.1.nip.io/v1", false)]
    [InlineData("http://localhost./v1", false)]
    [InlineData("http://0.0.0.0:8080/v1", false)]
    [InlineData("http://127.1/v1", false)]
    [InlineData("http://0127.0.0.1/v1", false)]
    [InlineData("http://[::ffff:127.0.0.1]/v1", false)]
    [InlineData("http://127.0.0.1@example.com/v1", false)]
    [InlineData("https://api.openai.com/v1", false)]
    [InlineData("localhost:8080", false)]
    [InlineData("", false)]
    public void OnThisPcFollowsTheCoresRule(string url, bool local) => Assert.Equal(local, CloudModel.IsOnThisPc(url));

    /// <summary>None stops using the provider: Use sends none, which turns local-only mode back on.</summary>
    [Fact]
    public void NoneStopsUsingTheProvider()
    {
        var (cloud, sent) = Loaded(Providers("openai", "gpt-4o-mini", "cloud", localOnly: false, ready: true, keyed: ["openai"]));
        Assert.Equal("openai", cloud.Selected); // the picker follows the choice
        cloud.Select(null);
        Assert.Equal("Stop using a language model", cloud.UseLabel);
        Assert.Equal("Stopping turns local-only mode back on: nothing you say leaves this PC.", cloud.UseNote);
        cloud.Use();
        Assert.Equal(new CoreCommand.LlmChoose("none", null, null, false, "llm.choose:2"), sent.Commands[^1]);
    }

    /// <summary>A chosen provider with no key, or held back by local-only mode, says why nothing can use it.</summary>
    [Fact]
    public void AChosenProviderThatCannotBeCalledSaysWhy()
    {
        var (cloud, _) = Loaded(Providers("anthropic", "claude-synthetic", "cloud", localOnly: false));
        Assert.Equal("claude-synthetic at Anthropic is chosen, but no key is stored, so nothing can use it.", cloud.Status);
        cloud.Apply(Providers("anthropic", "claude-synthetic", "cloud", localOnly: true, keyed: ["anthropic"]));
        Assert.Equal("claude-synthetic at Anthropic is chosen, but local-only mode is on, so nothing is sent.", cloud.Status);
        cloud.Apply(Providers(error: "couldn't read the stored keys"));
        Assert.Equal("Couldn't read the stored keys.", cloud.Status);
        Assert.True(cloud.IsProblem);
    }

    /// <summary>Test asks the core, shows its answer (only the newest test's), and a failure in words.</summary>
    [Fact]
    public void ATestShowsWhatTheProviderSaid()
    {
        var (cloud, sent) = Loaded(Providers("openai", "gpt-4o-mini", "cloud", localOnly: false, ready: true, keyed: ["openai"]));
        cloud.Test();
        Assert.Equal(new CoreCommand.LlmTest("llm.test:2"), sent.Commands[^1]);
        Assert.Equal(CloudTestState.Testing, cloud.TestState);
        Assert.False(cloud.CanTest); // one at a time
        cloud.Apply(Ev.Of("""{"type":"llm.tested","provider":"openai","model":"gpt-4o-mini","ok":false,"status":401,"error":"couldn't get an answer: the provider refused the key","ref":"llm.test:2"}"""));
        Assert.Equal(CloudTestState.Failed, cloud.TestState);
        Assert.Equal("Couldn't get an answer: the provider refused the key.", cloud.TestMessage);
        Assert.True(cloud.IsProblem);

        cloud.Test();
        cloud.Apply(Ev.Of("""{"type":"llm.tested","provider":"openai","model":"gpt-4o-mini","ok":false,"ref":"llm.test:2"}""")); // an older answer
        Assert.Equal(CloudTestState.Testing, cloud.TestState);
        cloud.Apply(Ev.Of("""{"type":"llm.tested","provider":"openai","model":"gpt-4o-mini","ok":true,"ref":"llm.test:3"}"""));
        Assert.Equal(CloudTestState.Passed, cloud.TestState);
        Assert.Equal("OpenAI answered with gpt-4o-mini.", cloud.TestMessage);

        cloud.Test();
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"llm.test","id":"llm.test:4","message":"a test is already running; wait for its answer"}""");
        Assert.True(CloudModel.Handles(failed));
        cloud.Apply(failed);
        Assert.Equal("Couldn't test it: a test is already running; wait for its answer.", cloud.TestMessage);
    }

    /// <summary>
    /// A test's answer is of the provider, model and key it was sent with: once any of them
    /// changes, a late answer is not shown under the new setup.
    /// </summary>
    [Fact]
    public void AStaleTestAnswerNeverWins()
    {
        const string LateOk = """{"type":"llm.tested","provider":"openai","model":"gpt-4o-mini","ok":true,"ref":"llm.test:2"}""";
        var (cloud, sent) = Loaded(Providers("openai", "gpt-4o-mini", "cloud", localOnly: false, ready: true, keyed: ["openai", "anthropic"]));
        cloud.Test();
        Assert.Equal(new CoreCommand.LlmTest("llm.test:2"), sent.Commands[^1]);
        cloud.Select("anthropic");
        cloud.Use();
        Assert.Equal(CloudTestState.None, cloud.TestState);
        cloud.Apply(Ev.Of(LateOk));
        Assert.Equal(CloudTestState.None, cloud.TestState);
        Assert.Null(cloud.TestMessage);

        foreach (var change in new Action<CloudModel>[] { c => c.SaveKey(Key), c => c.DeleteKey(), c => c.Select("openai") })
        {
            (cloud, _) = Loaded(Providers("openai", "gpt-4o-mini", "cloud", localOnly: false, ready: true, keyed: ["openai"]));
            cloud.Test();
            change(cloud);
            cloud.Apply(Ev.Of(LateOk));
            Assert.Equal(CloudTestState.None, cloud.TestState);
            Assert.Null(cloud.TestMessage);
        }
    }

    /// <summary>A refused command is said under the section, in words, never read as done.</summary>
    [Fact]
    public void ARefusedCommandIsSaid()
    {
        var (cloud, _) = Loaded();
        cloud.Select("openai");
        cloud.SaveKey(Key);
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"llm.key.save","id":"llm.key.save:2","message":"couldn't save the key: the key store refused"}""");
        Assert.True(CloudModel.Handles(failed));
        cloud.Apply(failed);
        Assert.Equal("Couldn't save the key: the key store refused.", cloud.Failure);
        cloud.Apply(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"llm.choose","id":"llm.choose:3","message":"llm.choose: the URL must start with http:// or https://"}"""));
        Assert.Equal("Couldn't do that: the URL must start with http:// or https://.", cloud.Failure);
        Assert.False(CloudModel.Handles(Ev.Of<CommandFailed>("""{"type":"command.failed","command":"setting.set","id":"setting:meetings.llm","message":"x"}""")));
    }

    /// <summary>Local-only mode changed elsewhere: the providers are read again (ready may have changed).</summary>
    [Fact]
    public void ALocalOnlyChangeReadsTheProvidersAgain()
    {
        var (cloud, sent) = Loaded();
        var before = sent.Commands.Count;
        cloud.Apply(Ev.Of("""{"type":"setting.value","key":"llm.local_only","value":"on"}"""));
        Assert.IsType<CoreCommand.LlmProviders>(sent.Commands[before]);
    }

    /// <summary>
    /// Credential Manager's entry is the Windows account's, shared by every Inkwell on it: a stored
    /// key says so, in Settings and in the first run, and Delete says whose key goes before it goes.
    /// </summary>
    [Fact]
    public void AStoredKeyIsSaidToBeTheAccountsAndDeleteSaysWhatItDeletes()
    {
        var (cloud, sent) = Loaded(Providers(keyed: ["groq"]));
        Assert.Equal(
            "A Groq key is already stored for this Windows account, in Windows Credential Manager: every Inkwell on this account can use it. Saving a new one replaces it.",
            OnboardingModel.StoredKeyLine(cloud));
        cloud.Select("groq");
        Assert.Equal("A key is stored for this Windows account, in Windows Credential Manager: every Inkwell on this account uses it, whichever library it opens.", cloud.KeyStatus);
        Assert.Equal("Delete Groq key", cloud.DeleteKeyLabel);
        Assert.Equal("Delete the Groq key stored for this Windows account?", cloud.DeleteKeyQuestion);
        Assert.Equal(
            "It is removed from Windows Credential Manager, so no Inkwell on this account can use it, whichever library it opens. Nothing else is deleted.",
            CloudModel.DeleteKeyDetail);
        var before = sent.Commands.Count;
        cloud.DeleteKey();
        Assert.Equal("groq", Assert.IsType<CoreCommand.LlmKeyDelete>(sent.Commands[^1]).Provider);
        Assert.Equal(before + 1, sent.Commands.Count);

        (cloud, _) = Loaded(Providers(keyed: []));
        Assert.Null(OnboardingModel.StoredKeyLine(cloud));
        cloud.Select(null);
        Assert.Equal("Delete key", cloud.DeleteKeyLabel);
    }

}

public class CloudPolishTests
{
    /// <summary>
    /// Windows has no model on the device: a chosen, ready own-key provider is the working model
    /// the AI switches need, and one that is not ready (no key, local-only on) leaves them off.
    /// </summary>
    [Fact]
    public void AReadyOwnKeyProviderIsAWorkingModel()
    {
        var screens = new SettingsHarness(_ => { });
        screens.Apply(State(on: false, allowed: false, to: null));
        Assert.False(screens.Polish.HasWorkingEngine);
        screens.Apply(
            CloudModelTests.Providers("openai", "gpt-4o-mini", "cloud", localOnly: false, ready: true, keyed: ["openai"]),
            State(on: false, allowed: false, to: "cloud", name: "gpt-4o-mini (openai)", endpoint: "https://api.openai.com/v1"),
            State(on: false, allowed: false, to: "cloud", name: "gpt-4o-mini (openai)", endpoint: "https://api.openai.com/v1", feature: "meetings"));
        Assert.True(screens.Polish.HasWorkingEngine);
        Assert.True(screens.Polish.CanToggle);
        Assert.True(screens.Ai.CanToggleMeetingsAI);
        // Turning a feature on still asks, naming the provider: choosing it sent nothing.
        screens.Polish.SetOn(true);
        var asked = Assert.IsType<ConsentDestination>(screens.Polish.PendingConsent);
        Assert.Equal("Send to gpt-4o-mini (openai)", ConsentModel.Button(LlmFeature.Polish, asked));

        screens.Apply(CloudModelTests.Providers("openai", "gpt-4o-mini", "cloud", localOnly: false, ready: false));
        Assert.False(screens.Polish.HasWorkingEngine);
        Assert.Equal(PolishModel.NoModelText, screens.Polish.Status);
    }
}
