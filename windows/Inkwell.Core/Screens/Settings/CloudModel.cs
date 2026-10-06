// Settings > AI's language model (Windows): this PC's own model (on_device: the core's, once
// downloaded; LocalLlmModel has its download), or an own-key (BYOK) provider (OpenAI, Anthropic,
// Groq, OpenRouter, or any OpenAI-compatible server), its API key, and a model. The Mac has no such
// screen.
//
// With no provider chosen, the core uses this PC's model once it is downloaded: the status says
// so, and Try it asks that model. Choosing a provider later takes over from it; choosing on_device
// keeps this PC's model even when a provider's key is stored, and while it is chosen nothing
// stands in for it (removed, the features have no model).
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
/// <param name="Installed">For this PC's model, whether it is downloaded; null for a provider.</param>
public sealed record CloudProvider(string Id, string Name, string DefaultModel, string Endpoint, bool CustomUrl, bool NeedsKey, bool HasKey, bool? Installed = null)
{
    /// <summary>This PC's own model (on_device), not a provider.</summary>
    public bool IsOnDevice => Id == CloudModel.OnDeviceId;
}

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
    /// <summary>The newest Use of this PC's model: its answer raises <see cref="ChoseOnDevice"/>.</summary>
    private string? onDeviceChooseRef;
    /// <summary>The newest answer chose this PC's model in reply to its Use: raised after the change.</summary>
    private bool choseOnDevice;
    /// <summary>The newest test's ref: only its answer is shown, and none once the provider, model or key changed.</summary>
    private string? testRef;

    public CloudModel(Action<CoreCommand> send)
    {
        ArgumentNullException.ThrowIfNull(send);
        this.send = send;
    }

    /// <summary>llm.choose's provider, and llm.providers' entry, for this PC's own model.</summary>
    public const string OnDeviceId = "on_device";

    /// <summary>
    /// Raised once the core has chosen this PC's model in answer to its Use (the features' states
    /// arrive before it): Settings > AI then asks polish's one-tap consent (LocalLlmModel).
    /// </summary>
    public event Action? ChoseOnDevice;

    /// <summary>The name of the language model with a registry id (the catalogue's), for this PC's model's lines.</summary>
    public Func<string, string?> ModelName { get; set; } = _ => null;

    // What the core says.

    /// <summary>The providers, in the core's order (empty until read); this PC's model among them where the core offers one.</summary>
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

    /// <summary>This PC's own model, where the core offers one (Windows).</summary>
    public CloudProvider? OnDevice => Providers.FirstOrDefault(p => p.IsOnDevice);

    /// <summary>The own-key providers (the first run's other providers).</summary>
    public IEnumerable<CloudProvider> OwnKeyProviders => Providers.Where(p => !p.IsOnDevice);

    /// <summary>This PC's model is downloaded.</summary>
    public bool OnDeviceInstalled => OnDevice?.Installed == true;

    /// <summary>
    /// What the features use is this PC's model: chosen and downloaded, or downloaded with no
    /// provider chosen (the core then uses it).
    /// </summary>
    public bool OnDeviceInUse => OnDeviceInstalled && ReadError is null && (Chosen == OnDeviceId || Chosen is null);

    /// <summary>This PC's model's name: the catalogue's (Qwen3 4B Instruct), else "this PC's model".</summary>
    public string OnDeviceName => (ChosenModel is string chosen && Chosen == OnDeviceId ? ModelName(chosen) : null)
        ?? (OnDevice is CloudProvider local ? ModelName(local.DefaultModel) : null)
        ?? "this PC's model";

    /// <summary>A name for people.</summary>
    public static string ProviderName(string id) => id switch
    {
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "groq" => "Groq",
        "openrouter" => "OpenRouter",
        "custom" => "OpenAI-compatible server",
        OnDeviceId => "This PC's model",
        _ => id,
    };

    /// <summary>Whether the provider in the picker, with the address typed, is off this PC (this PC's own model never is).</summary>
    public bool SelectedIsCloud => SelectedProvider is CloudProvider p && !p.IsOnDevice && (!p.CustomUrl || !IsOnThisPc(BaseUrlFor(p)));

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
            if (p.IsOnDevice)
            {
                // Only once it is downloaded: the core refuses to choose a model it doesn't have.
                return p.Installed == true && Chosen != OnDeviceId;
            }
            if (p.CustomUrl && string.IsNullOrWhiteSpace(DraftBaseUrl))
            {
                return false;
            }
            return p.Id != Chosen || ModelFor(p) != ChosenModel || (p.CustomUrl && BaseUrlFor(p) != ChosenBaseUrl);
        }
    }

    /// <summary>
    /// Whether Test (Try it, for this PC's model) can be pressed: the provider in the picker is
    /// what the features use (chosen, or this PC's model downloaded with none chosen), and no test
    /// is running.
    /// </summary>
    public bool CanTest => TestState != CloudTestState.Testing
        && ((Chosen is not null && Selected == Chosen) || (Selected == OnDeviceId && Chosen is null && OnDeviceInstalled));

    /// <summary>What the Use button says.</summary>
    public string UseLabel => SelectedProvider is CloudProvider p
        ? p.IsOnDevice ? "Use this PC's model" : $"Use {ProviderName(p.Id)}"
        : "Stop using a language model";

    /// <summary>What the Test button says: Try it for this PC's model, which loads it first.</summary>
    public string TestLabel => SelectedProvider?.IsOnDevice == true ? "Try it" : "Test";

    /// <summary>The Test button's name for Narrator.</summary>
    public string TestName => SelectedProvider?.IsOnDevice == true ? $"Try {OnDeviceName} on this PC" : "Test the language model";

    /// <summary>What choosing the provider in the picker means, said before the user presses Use.</summary>
    public string UseNote
    {
        get
        {
            if (SelectedProvider is not CloudProvider p)
            {
                if (Chosen is null)
                {
                    return OnDeviceInstalled
                        ? $"No provider is chosen, so the features below use {OnDeviceName}, on this PC. Nothing you say leaves this PC."
                        : "No language model is chosen. Nothing you say leaves this PC.";
                }
                return OnDeviceInstalled
                    ? $"Stopping goes back to {OnDeviceName}, on this PC, and turns local-only mode back on: nothing you say leaves this PC."
                    : "Stopping turns local-only mode back on: nothing you say leaves this PC.";
            }
            if (p.IsOnDevice)
            {
                if (p.Installed != true)
                {
                    return "Download it first. It runs on this PC, so nothing you say leaves it.";
                }
                return Chosen == OnDeviceId
                    ? $"{Sentence(OnDeviceName)} is in use. It runs on this PC, so local-only mode stays on and your words stay here."
                    : $"{Sentence(OnDeviceName)} runs on this PC, so local-only mode stays on and your words stay here. Use asks once before polish uses it.";
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
            if (SelectedProvider is not CloudProvider p || p.IsOnDevice)
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
            if (Chosen == OnDeviceId)
            {
                return OnDeviceInstalled && Ready
                    ? $"In use: {OnDeviceName}, on this PC. Local-only mode is on."
                    : "This PC's model is chosen, but it isn't downloaded, so nothing can use it. Download it, or choose another.";
            }
            if (ChosenProvider is not CloudProvider p)
            {
                return OnDeviceInstalled
                    ? $"In use: {OnDeviceName}, on this PC, as no provider is chosen. Local-only mode is on."
                    : "No language model is in use. Local-only mode is on.";
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
        // Only the newest Use's answer can ask polish's one tap.
        onDeviceChooseRef = null;
        if (SelectedProvider is not CloudProvider p)
        {
            send(new CoreCommand.LlmChoose("none", null, null, false, NextRef("choose")));
        }
        else if (p.IsOnDevice)
        {
            // Whichever model is downloaded: no model named, no address, local-only mode stays on.
            onDeviceChooseRef = NextRef("choose");
            send(new CoreCommand.LlmChoose(OnDeviceId, null, null, false, onDeviceChooseRef));
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
        // This PC's model in the picker is no own key picked: the first run's own key still starts on Groq.
        if (Chosen is null or OnDeviceId && Selected is null or OnDeviceId && Providers.Any(p => p.Id == id))
        {
            Select(id);
        }
    }

    /// <summary>
    /// The first run's own key is one choice, Groq's free model (its key and Use); another provider
    /// or model is behind "Other providers or models…". Those open first when another provider is
    /// chosen, or picked here or in Settings > AI: the user's pick stands.
    /// </summary>
    public bool FirstRunStartsOnOthers => Selected is not null && Selected != "groq" && Selected != OnDeviceId;

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
        TestMessage = Selected == OnDeviceId ? $"Loading {OnDeviceName} and asking it…" : $"Asking {ProviderName(Chosen!)}…";
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
        if (choseOnDevice)
        {
            choseOnDevice = false;
            ChoseOnDevice?.Invoke();
        }
    }

    private bool Fold(InkEvent e)
    {
        switch (e)
        {
            case LlmProviders state:
                var first = !Loaded;
                // The own-key providers, and this PC's model where the core offers one.
                Providers = state.Providers
                    .Select(p => new CloudProvider(p.Id, ProviderName(p.Id), p.DefaultModel, p.Endpoint, p.CustomUrl, p.NeedsKey, p.HasKey, p.Installed))
                    .ToList();
                var chosen = state.Chosen;
                // The answer to this PC's model's Use, having chosen it: polish's one tap follows.
                if (state.Ref is not null && state.Ref == onDeviceChooseRef)
                {
                    onDeviceChooseRef = null;
                    choseOnDevice = chosen == OnDeviceId;
                }
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
                TestMessage = !tested.Ok
                    ? $"{Sentence(tested.Error ?? "couldn't get an answer")}."
                    : tested.Provider == OnDeviceId
                        ? OnDeviceAnswered(tested)
                        : $"{ProviderName(tested.Provider)} answered with {tested.Model}.";
                return true;
            case CommandFailed failed when Handles(failed):
                if (failed.Id is not null && failed.Id == onDeviceChooseRef)
                {
                    // That Use was refused (nothing downloaded, say): no one tap follows it.
                    onDeviceChooseRef = null;
                }
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

    /// <summary>
    /// Try it's answer from this PC's model: "Qwen3 4B Instruct loaded in 1.2 s and answered in
    /// 0.4 s." (a load near 0 when it was in memory already). A dictation's polish reads and writes more.
    /// </summary>
    private string OnDeviceAnswered(LlmTested tested)
    {
        var name = Sentence(ModelName(tested.Model) ?? OnDeviceName);
        var answered = tested.AnswerMs is long answer ? $"answered in {Seconds(answer)}" : "answered";
        return tested.LoadMs is long load
            ? $"{name} loaded in {Seconds(load)} and {answered}."
            : $"{name} {answered}.";
    }

    /// <summary>Milliseconds as seconds, one decimal: "1.2 s".</summary>
    public static string Seconds(long ms) =>
        string.Format(System.Globalization.CultureInfo.CurrentCulture, "{0:0.0} s", Math.Max(0, ms) / 1000.0);

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
