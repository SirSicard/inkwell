// Settings > AI's language model (Windows): an own-key (BYOK) provider. Windows has no language
// model on the device, so polish, voice edit, summaries and Ask need one the user brings: a
// provider (OpenAI, Anthropic, Groq, OpenRouter, or any OpenAI-compatible server), its API key,
// and a model. The Mac has no such screen.
//
// The core holds everything: llm.providers says which providers there are, whether each has a key
// stored (asked without reading it), which one is chosen and whether it is ready. The key goes
// once, in llm.key.save, into Windows Credential Manager; this model never keeps it, and nothing
// here logs it (CoreCommand.LlmKeySave's ToString hides it too).
//
// Choosing a provider that is not on this PC turns local-only mode off, and the step says so
// before the user presses Use; the command carries the user's say-so ("local_only":"off"), and the
// core refuses such a choice without it. Choosing sends nothing: each feature below sends only with
// its own consent for this provider (ConsentModel), and the core refuses to send without it. A
// consent is kept per address, so one given before sends again as soon as that provider is chosen
// again; the note under Use says so.
//
// Test sends one short fixed request (never the user's words) to the chosen provider with its key,
// with no feature's consent: the section's caption says so before it is pressed.
// A failure of any command is said under the section ("Couldn't ..."), never read as success.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>A provider as the picker shows it.</summary>
/// <param name="Id">The core's id (openai, groq, anthropic, openrouter, custom).</param>
/// <param name="Name">Its name for people.</param>
/// <param name="DefaultModel">The model used when none is named.</param>
/// <param name="Endpoint">Its address (for custom, the default one).</param>
/// <param name="CustomUrl">Whether the user names the address (custom only).</param>
/// <param name="NeedsKey">Whether it needs an API key.</param>
/// <param name="HasKey">Whether a key is stored for it.</param>
public sealed record CloudProvider(string Id, string Name, string DefaultModel, string Endpoint, bool CustomUrl, bool NeedsKey, bool HasKey);

/// <summary>Where a key test is.</summary>
public enum CloudTestState
{
    None,
    Testing,
    Passed,
    Failed,
}

public sealed class CloudModel : ObservableModel
{
    private readonly Action<CoreCommand> send;
    private int requests;
    /// <summary>The newest test's ref: only its answer is shown, and none once the provider, model or key changed.</summary>
    private string? testRef;

    public CloudModel(Action<CoreCommand> send)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
    }

    // What the core says.

    /// <summary>The providers, in the core's order (empty until read).</summary>
    public IReadOnlyList<CloudProvider> Providers { get; private set; } = [];

    /// <summary>Whether the core has answered once.</summary>
    public bool Loaded { get; private set; }

    /// <summary>The chosen provider's id (null: none).</summary>
    public string? Chosen { get; private set; }

    /// <summary>The model the chosen provider is asked for.</summary>
    public string? ChosenModel { get; private set; }

    /// <summary>For a custom server, its address as chosen.</summary>
    public string? ChosenBaseUrl { get; private set; }

    /// <summary>Where the chosen provider sends.</summary>
    public string? ChosenEndpoint { get; private set; }

    /// <summary>Whether the chosen provider is off this PC.</summary>
    public bool ChosenIsCloud { get; private set; }

    /// <summary>Local-only mode: on unless a cloud provider was chosen.</summary>
    public bool LocalOnly { get; private set; } = true;

    /// <summary>Whether the chosen provider can be called (its key stored, local-only mode letting it through).</summary>
    public bool Ready { get; private set; }

    /// <summary>What the core could not read ("couldn't read ...").</summary>
    public string? ReadError { get; private set; }

    // The picker (the provider being set up: the chosen one until the user picks another).

    /// <summary>The provider in the picker (null: none).</summary>
    public string? Selected { get; private set; }

    /// <summary>The model in the picker; empty for the provider's default.</summary>
    public string DraftModel { get; set; } = "";

    /// <summary>A custom server's address in the picker.</summary>
    public string DraftBaseUrl { get; set; } = "";

    /// <summary>A command that failed, as a sentence ("Couldn't save the key: ...").</summary>
    public string? Failure { get; private set; }

    public CloudTestState TestState { get; private set; }

    /// <summary>What the last test said.</summary>
    public string? TestMessage { get; private set; }

    /// <summary>The provider in the picker, if it is one the core listed.</summary>
    public CloudProvider? SelectedProvider => Providers.FirstOrDefault(p => p.Id == Selected);

    /// <summary>The chosen provider, if any.</summary>
    public CloudProvider? ChosenProvider => Providers.FirstOrDefault(p => p.Id == Chosen);

    /// <summary>A name for people.</summary>
    public static string ProviderName(string id) => id switch
    {
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "groq" => "Groq",
        "openrouter" => "OpenRouter",
        "custom" => "OpenAI-compatible server",
        _ => id,
    };

    /// <summary>Whether the provider in the picker, with the address typed, is off this PC.</summary>
    public bool SelectedIsCloud => SelectedProvider is CloudProvider p && (!p.CustomUrl || !IsOnThisPc(BaseUrlFor(p)));

    /// <summary>Whether Use would change anything: another provider, model or address than the chosen one.</summary>
    public bool CanUse
    {
        get
        {
            if (!Loaded)
            {
                return false;
            }
            if (SelectedProvider is not CloudProvider p)
            {
                return Selected is null && Chosen is not null;
            }
            if (p.CustomUrl && string.IsNullOrWhiteSpace(DraftBaseUrl))
            {
                return false;
            }
            return p.Id != Chosen || ModelFor(p) != ChosenModel || (p.CustomUrl && BaseUrlFor(p) != ChosenBaseUrl);
        }
    }

    /// <summary>Whether Test can be pressed: the provider in the picker is the chosen one, and no test is running.</summary>
    public bool CanTest => Chosen is not null && Selected == Chosen && TestState != CloudTestState.Testing;

    /// <summary>What the Use button says.</summary>
    public string UseLabel => SelectedProvider is CloudProvider p ? $"Use {ProviderName(p.Id)}" : "Stop using a language model";

    /// <summary>What choosing the provider in the picker means, said before the user presses Use.</summary>
    public string UseNote
    {
        get
        {
            if (SelectedProvider is not CloudProvider p)
            {
                return Chosen is null
                    ? "No language model is chosen. Nothing you say leaves this PC."
                    : "Stopping turns local-only mode back on: nothing you say leaves this PC.";
            }
            var name = ProviderName(p.Id);
            return SelectedIsCloud
                ? $"Using {name} turns local-only mode off, so the features below can send to {name}. Each one sends only once you allow it for {name}; one you already allowed for {name} sends again straight away."
                : $"This server is on this PC, so local-only mode stays on. Each feature below uses it only once you allow it; one you already allowed uses it straight away.";
        }
    }

    /// <summary>
    /// Whether the core keeps the key back from the server in the picker: a custom server over
    /// plain http that is not on this PC gets no key, stored or not (the core's rule).
    /// </summary>
    public bool KeyWithheld => SelectedProvider is { CustomUrl: true } p && KeyWithheldFrom(BaseUrlFor(p));

    /// <summary>The key's name for people: "Groq key"; a server of the user's own has a "server key".</summary>
    public static string KeyName(CloudProvider provider)
    {
        ArgumentNullException.ThrowIfNull(provider);
        return provider.CustomUrl ? "server key" : $"{ProviderName(provider.Id)} key";
    }

    /// <summary>"A Groq key", "An OpenAI key".</summary>
    private static string AKey(CloudProvider provider)
    {
        var key = KeyName(provider);
        return ("aeiou".Contains(char.ToLowerInvariant(key[0]), StringComparison.Ordinal) ? "An " : "A ") + key;
    }

    /// <summary>
    /// The line about the key of the provider in the picker (the Mac's words). Keys are in Windows
    /// Credential Manager for the Windows account, shared by every Inkwell on it and never in a
    /// library: a scratch library sees the user's own key, so the line says whose it is.
    /// </summary>
    public string KeyStatus
    {
        get
        {
            if (SelectedProvider is not CloudProvider p)
            {
                return "";
            }
            if (p.HasKey && KeyWithheld)
            {
                return $"{AKey(p)} is already saved for this Windows account, but it is not sent to this server: keys go only over https or to a server on this PC.";
            }
            if (KeyWithheld)
            {
                return "No key is sent to this server: keys go only over https or to a server on this PC.";
            }
            if (!p.NeedsKey && !p.HasKey)
            {
                return "No key is needed unless your server asks for one.";
            }
            return p.HasKey ? $"{AKey(p)} is already saved for this Windows account." : $"No {KeyName(p)} is saved yet.";
        }
    }

    /// <summary>The Delete button of the provider in the picker: what it deletes, and that it asks first.</summary>
    public string DeleteKeyLabel => SelectedProvider is CloudProvider p ? $"Delete {KeyName(p)}\u2026" : "Delete key\u2026";

    /// <summary>The question Delete asks about the provider in the picker's key.</summary>
    public string DeleteKeyQuestion => SelectedProvider is CloudProvider p
        ? $"Delete the {KeyName(p)} from this Windows account?"
        : "Delete the key from this Windows account?";

    /// <summary>What Delete deletes, said with the question.</summary>
    public const string DeleteKeyDetail =
        "It is saved for this Windows account, not in this library: every Inkwell on this account stops using it. You can paste it again later.";

    /// <summary>The question's buttons.</summary>
    public const string DeleteKeyConfirm = "Delete Key";

    /// <summary>Whether the core keeps a key back from <paramref name="url"/>: plain http to a server that is not on this PC.</summary>
    public static bool KeyWithheldFrom(string url)
    {
        ArgumentNullException.ThrowIfNull(url);
        return url.Trim().StartsWith("http://", StringComparison.OrdinalIgnoreCase) && !IsOnThisPc(url);
    }

    /// <summary>The line about what is in use now.</summary>
    public string Status
    {
        get
        {
            if (ReadError is string error)
            {
                return $"{Sentence(error)}.";
            }
            if (!Loaded)
            {
                return "Reading the language model settings…";
            }
            if (ChosenProvider is not CloudProvider p)
            {
                return "No language model is in use. Local-only mode is on.";
            }
            var name = ProviderName(p.Id);
            var where = ChosenIsCloud ? name : $"{name}, on this PC";
            if (p.NeedsKey && !p.HasKey)
            {
                return $"{ChosenModel} at {where} is chosen, but no key is stored, so nothing can use it.";
            }
            if (!Ready)
            {
                return $"{ChosenModel} at {where} is chosen, but local-only mode is on, so nothing is sent.";
            }
            return ChosenIsCloud
                ? $"In use: {ChosenModel} at {name}. Local-only mode is off."
                : $"In use: {ChosenModel} at {where}. Local-only mode is on.";
        }
    }

    /// <summary>Whether a problem is shown (alert colour).</summary>
    public bool IsProblem => Failure is not null || ReadError is not null || TestState == CloudTestState.Failed;

    // Commands.

    private string NextRef(string kind)
    {
        requests++;
        return $"{RefPrefix}{kind}:{requests}";
    }

    /// <summary>Reads the providers from the core.</summary>
    public void Load() => send(new CoreCommand.LlmProviders(NextRef("providers")));

    /// <summary>The user picked a provider (null: none) in the picker. Nothing is sent.</summary>
    public void Select(string? id)
    {
        ForgetTest();
        Selected = id;
        DraftModel = id is not null && id == Chosen ? ChosenModel ?? "" : "";
        DraftBaseUrl = id is not null && id == Chosen ? ChosenBaseUrl ?? "" : "";
        if (id is not null && DraftBaseUrl.Length == 0 && SelectedProvider is { CustomUrl: true } p)
        {
            DraftBaseUrl = p.Endpoint;
        }
        Failure = null;
        Changed();
    }

    /// <summary>
    /// Stores <paramref name="key"/> for the provider in the picker. The key is sent once and not
    /// kept here: the view clears its box after this.
    /// </summary>
    public void SaveKey(string key)
    {
        ArgumentNullException.ThrowIfNull(key);
        if (SelectedProvider is not CloudProvider p)
        {
            return;
        }
        if (key.Trim().Length == 0)
        {
            Failure = "Type or paste the key first.";
            Changed();
            return;
        }
        Failure = null;
        ForgetTest();
        send(new CoreCommand.LlmKeySave(p.Id, key, NextRef("key.save")));
        Changed();
    }

    /// <summary>Deletes the stored key of the provider in the picker.</summary>
    public void DeleteKey()
    {
        if (SelectedProvider is not CloudProvider p)
        {
            return;
        }
        Failure = null;
        ForgetTest();
        send(new CoreCommand.LlmKeyDelete(p.Id, NextRef("key.delete")));
        Changed();
    }

    /// <summary>
    /// Use: chooses the provider in the picker with its model (and address), or none. For a
    /// provider off this PC it carries the user's say-so that local-only mode goes off. Whether
    /// the choice was sent.
    /// </summary>
    public bool Use()
    {
        if (!CanUse)
        {
            return false;
        }
        Failure = null;
        ForgetTest();
        if (SelectedProvider is not CloudProvider p)
        {
            send(new CoreCommand.LlmChoose("none", null, null, false, NextRef("choose")));
        }
        else
        {
            var model = DraftModel.Trim();
            send(new CoreCommand.LlmChoose(
                p.Id, model.Length == 0 ? null : model, p.CustomUrl ? BaseUrlFor(p) : null, SelectedIsCloud, NextRef("choose")));
        }
        Changed();
        return true;
    }

    /// <summary>
    /// Puts <paramref name="id"/> in the picker while nothing is chosen or picked (the first run
    /// points at Groq's free key). Nothing is sent.
    /// </summary>
    public void Suggest(string id)
    {
        if (Chosen is null && Selected is null && Providers.Any(p => p.Id == id))
        {
            Select(id);
        }
    }

    /// <summary>
    /// The first run's own key is one choice, Groq's free model (its key and Use); another provider
    /// or model is behind "Other providers or models…". Those open first when another provider is
    /// chosen, or picked here or in Settings > AI: the user's pick stands.
    /// </summary>
    public bool FirstRunStartsOnOthers => Selected is not null && Selected != "groq";

    /// <summary>Back from the other providers to Groq's free model: Groq in the picker. Nothing is sent.</summary>
    public void PickGroq()
    {
        if (Providers.Any(p => p.Id == "groq"))
        {
            Select("groq");
        }
    }

    /// <summary>Where the provider in the picker would send: its address, as the consent step names it.</summary>
    public string? SelectedEndpoint => SelectedProvider is CloudProvider p
        ? (p.CustomUrl ? BaseUrlFor(p).TrimEnd('/') : p.Endpoint)
        : null;

    /// <summary>What Use means in the first run, where it asks polish's consent before choosing.</summary>
    public string FirstRunUseNote
    {
        get
        {
            if (SelectedProvider is not CloudProvider p)
            {
                return "Pick a provider to bring your own key.";
            }
            var name = ProviderName(p.Id);
            if (p.NeedsKey && !p.HasKey)
            {
                return $"Save your {name} key first.";
            }
            return SelectedIsCloud
                ? $"Use asks first: polish sends your words to {name} only once you allow it, which also turns Local only off."
                : "This server is on this PC, so Local only stays on. Use asks before polish uses it.";
        }
    }

    /// <summary>Test: one short fixed request to the chosen provider.</summary>
    public void Test()
    {
        if (!CanTest)
        {
            return;
        }
        testRef = NextRef("test");
        TestState = CloudTestState.Testing;
        TestMessage = $"Asking {ProviderName(Chosen!)}…";
        Changed();
        send(new CoreCommand.LlmTest(testRef));
    }

    /// <summary>
    /// The provider, model or key changed: a test in flight or done was of what came before, so
    /// its answer is not shown (a late one no longer matches).
    /// </summary>
    private void ForgetTest()
    {
        testRef = null;
        TestState = CloudTestState.None;
        TestMessage = null;
    }

    /// <summary>Every ref this model sends starts with this.</summary>
    private const string RefPrefix = "llm.";

    /// <summary>Whether this model shows <paramref name="failed"/>: its own commands (their ids are its refs).</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return (failed.Command.StartsWith("llm.", StringComparison.Ordinal)
            && (failed.Id?.StartsWith(RefPrefix, StringComparison.Ordinal) ?? false))
            || (failed.Command == "setting.set" && failed.Id == ShellSetting.LlmLocalOnly.CommandId());
    }

    /// <summary>The Local only switch's caption.</summary>
    public const string LocalOnlyTitle = "Nothing leaves this computer";

    /// <summary>
    /// The Local only switch: on, no language model off this computer is called, whatever is
    /// chosen; off, the chosen provider may be. The core answers with setting.value, and the
    /// providers are read again (Apply).
    /// </summary>
    public void SetLocalOnly(bool on)
    {
        if (on == LocalOnly)
        {
            return;
        }
        Failure = null;
        LocalOnly = on;
        send(new CoreCommand.SettingSet(ShellSetting.LlmLocalOnly, on ? "on" : "off"));
        Changed();
    }

    public void Apply(InkEvent e)
    {
        if (Fold(e))
        {
            Changed();
        }
    }

    private bool Fold(InkEvent e)
    {
        switch (e)
        {
            case LlmProviders state:
                var first = !Loaded;
                // The own-key providers. This PC's model (on_device) is chosen from its own row,
                // which comes with the local language model UI.
                Providers = state.Providers
                    .Where(p => p.Id != "on_device")
                    .Select(p => new CloudProvider(p.Id, ProviderName(p.Id), p.DefaultModel, p.Endpoint, p.CustomUrl, p.NeedsKey, p.HasKey))
                    .ToList();
                // This PC's model chosen reads as no own-key provider chosen here: the local
                // language model UI takes it over.
                var chosen = state.Chosen == "on_device" ? null : state.Chosen;
                var choiceChanged = Chosen != chosen || ChosenModel != state.Model || ChosenBaseUrl != state.BaseUrl;
                Chosen = chosen;
                ChosenModel = state.Model;
                ChosenBaseUrl = state.BaseUrl;
                ChosenEndpoint = state.Endpoint;
                ChosenIsCloud = state.To == LlmDestination.Cloud;
                LocalOnly = state.LocalOnly;
                Ready = state.Ready;
                ReadError = state.Error;
                Loaded = true;
                // The picker follows the choice when it is first read and whenever it changes.
                if (first || choiceChanged)
                {
                    Select(Chosen);
                }
                return true;
            case LlmTested tested when tested.Ref is not null && tested.Ref == testRef:
                TestState = tested.Ok ? CloudTestState.Passed : CloudTestState.Failed;
                TestMessage = tested.Ok
                    ? $"{ProviderName(tested.Provider)} answered with {tested.Model}."
                    : $"{Sentence(tested.Error ?? "couldn't get an answer")}.";
                return true;
            case CommandFailed failed when Handles(failed):
                if (failed.Command == "llm.test")
                {
                    if (failed.Id != testRef)
                    {
                        return false;
                    }
                    TestState = CloudTestState.Failed;
                    TestMessage = $"Couldn't test it: {failed.Message}.";
                }
                else
                {
                    // The core's words, without the command's name it starts a refusal with.
                    var message = failed.Message.StartsWith($"{failed.Command}: ", StringComparison.Ordinal)
                        ? failed.Message[(failed.Command.Length + 2)..]
                        : failed.Message;
                    Failure = message.StartsWith("couldn't", StringComparison.Ordinal)
                        ? $"{Sentence(message)}."
                        : $"Couldn't do that: {message}.";
                    if (failed.Command == "setting.set")
                    {
                        // The Local only switch: show what the core holds.
                        Load();
                    }
                }
                return true;
            case SettingValue { Key: "llm.local_only" }:
                // Local-only mode changed: whether the chosen provider can be called changed with it.
                Load();
                return false;
            case CoreStopped:
                if (TestState == CloudTestState.Testing)
                {
                    TestState = CloudTestState.None;
                    TestMessage = null;
                    return true;
                }
                return false;
            default:
                return false;
        }
    }

    private string ModelFor(CloudProvider p)
    {
        var model = DraftModel.Trim();
        return model.Length == 0 ? p.DefaultModel : model;
    }

    private string BaseUrlFor(CloudProvider p)
    {
        var url = DraftBaseUrl.Trim();
        return url.Length == 0 ? p.Endpoint : url;
    }

    /// <summary>
    /// Whether a server address is on this PC, by the core's rule: host localhost, an IPv4 address
    /// in 127.0.0.0/8 written as four plain numbers, or [::1]. Anything else, or anything that does
    /// not read, is off this PC (the core decides in the end, and refuses a choice that disagrees).
    /// </summary>
    public static bool IsOnThisPc(string url)
    {
        ArgumentNullException.ThrowIfNull(url);
        var rest = url.Trim();
        var scheme = rest.IndexOf("://", StringComparison.Ordinal);
        if (scheme <= 0 || rest[..scheme].ToLowerInvariant() is not ("http" or "https"))
        {
            return false;
        }
        rest = rest[(scheme + 3)..];
        var end = rest.IndexOfAny(['/', '?', '#']);
        var authority = end < 0 ? rest : rest[..end];
        if (authority.Contains('@', StringComparison.Ordinal))
        {
            return false;
        }
        string host;
        if (authority.StartsWith('['))
        {
            var close = authority.IndexOf(']', StringComparison.Ordinal);
            if (close < 0)
            {
                return false;
            }
            host = authority[1..close];
            return System.Net.IPAddress.TryParse(host, out var v6)
                && v6.AddressFamily == System.Net.Sockets.AddressFamily.InterNetworkV6
                && !v6.IsIPv4MappedToIPv6
                && System.Net.IPAddress.IsLoopback(v6);
        }
        var colon = authority.IndexOf(':', StringComparison.Ordinal);
        host = colon < 0 ? authority : authority[..colon];
        if (host.Equals("localhost", StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }
        var parts = host.Split('.');
        return parts.Length == 4
            && parts.All(p => p.Length is > 0 and <= 3 && p.All(char.IsAsciiDigit) && (p.Length == 1 || p[0] != '0') && int.Parse(p, System.Globalization.CultureInfo.InvariantCulture) <= 255)
            && parts[0] == "127";
    }

    /// <summary>"couldn't x" as the start of a sentence.</summary>
    private static string Sentence(string s) => s.Length == 0 ? s : char.ToUpperInvariant(s[0]) + s[1..];
}
