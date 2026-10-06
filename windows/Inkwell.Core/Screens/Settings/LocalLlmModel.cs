// Settings > AI's "On this PC" entry (Windows): the core's own language model, downloaded once, on
// which polish, voice edit, summaries and Ask run without a key and with local-only mode on. One
// model, no size to pick. Its download, Cancel and Remove are the catalogue's (CatalogueModel: the
// same row Settings > Models shows); its Use and Try it are CloudModel's (llm.choose on_device and
// llm.test); this joins them and asks polish's one tap.
//
// Use chooses it first (that sends nothing anywhere: the model is in this process, and local-only
// mode stays on), then, once the core has chosen it, shows polish's consent step for it, unless
// polish is on with it already: "Turn on polish? … your words stay on this PC", one tap. Cancel
// leaves the model chosen and polish off. Voice edit and summaries each ask with their own switch.
//
// With no provider chosen the core uses this model as soon as it is downloaded; a provider chosen
// keeps its place. When the model comes or goes, where each feature would send changes without a
// llm.choose: the features' states and the providers are read again.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

public sealed class LocalLlmModel : ObservableModel
{
    private readonly CatalogueModel catalogue;
    private readonly CloudModel cloud;
    private readonly AiSettings ai;

    public LocalLlmModel(CatalogueModel catalogue, CloudModel cloud, AiSettings ai)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        ArgumentNullException.ThrowIfNull(cloud);
        ArgumentNullException.ThrowIfNull(ai);
        this.catalogue = catalogue;
        this.cloud = cloud;
        this.ai = ai;
        catalogue.PropertyChanged += (_, _) => Changed();
        cloud.PropertyChanged += (_, _) => Changed();
        cloud.ChoseOnDevice += AskPolish;
    }

    /// <summary>The model's row in the catalogue; null when the catalogue has none.</summary>
    public ModelRow? Row => catalogue.LanguageRow;

    /// <summary>Whether the entry shows: the core offers this PC's model and the catalogue lists it.</summary>
    public bool Offered => cloud.OnDevice is not null && Row is not null;

    /// <summary>Its name (Qwen3 4B Instruct).</summary>
    public string Name => Row?.Name ?? cloud.OnDeviceName;

    /// <summary>"Qwen3 4B Instruct · Apache-2.0 · 2.32 GB · from huggingface.co".</summary>
    public string? Line(IFormatProvider? format = null) => Row?.Line(format);

    /// <summary>Its files are on this PC.</summary>
    public bool Installed => Row?.Installed == true;

    /// <summary>The features use it now (chosen, or downloaded with no provider chosen).</summary>
    public bool InUse => Installed && cloud.OnDeviceInUse;

    public bool CanDownload => Row?.CanDownload == true;

    public bool CanRetry => Row?.CanRetry == true;

    public bool CanCancel => Row?.CanCancel == true;

    public bool CanRemove => Row?.CanRemove == true;

    /// <summary>How far the download has got, 0 to 1, while it runs.</summary>
    public double? Progress => Row?.Progress;

    /// <summary>The line under it: the download's state or its failure, the last Cancel's or Remove's note, else whether it is in.</summary>
    public string Status(IFormatProvider? format = null)
    {
        if (Row is not ModelRow row)
        {
            return "This PC's model isn't listed yet.";
        }
        if (row.Note is string note)
        {
            return note;
        }
        if (row.Status(format) is string status)
        {
            return status;
        }
        if (!row.Installed)
        {
            return "Not downloaded. Nothing downloads until you press Download.";
        }
        if (InUse)
        {
            return "Downloaded and in use. Your words stay on this PC.";
        }
        return cloud.Chosen is string chosen && chosen != CloudModel.OnDeviceId
            ? $"Downloaded. {CloudModel.ProviderName(chosen)} is chosen, so this one isn't in use until you press Use."
            : "Downloaded.";
    }

    /// <summary>Whether the line under it is a problem (alert colour).</summary>
    public bool IsProblem => Row is { } row && (row.CanRetry || row.Note is not null);

    /// <summary>The free space on the volume models go on ("47.2 GB free on C:"), while it is not in.</summary>
    public string? FreeSpaceText(IFormatProvider? format = null) => Installed ? null : catalogue.FreeSpaceText(format);

    public void Download()
    {
        if (Row is ModelRow row)
        {
            catalogue.Download(row.Id);
        }
    }

    public void Cancel()
    {
        if (Row is ModelRow row)
        {
            catalogue.Cancel(row.Id);
        }
    }

    /// <summary>Remove, once the user confirmed it (ModelRow.RemoveQuestion).</summary>
    public void Remove()
    {
        if (Row is ModelRow row)
        {
            catalogue.Remove(row.Id);
        }
    }

    /// <summary>
    /// After the core chose this PC's model in answer to its Use: polish's one tap, when polish is
    /// not on with it already. Only for a step that names this PC (the features' states arrive
    /// before the choice's answer): never a step for anywhere else.
    /// </summary>
    private void AskPolish()
    {
        var consent = ai.Polish.Consent;
        if (consent.Destination is not { IsOnDevice: true } || consent.IsAllowedOn)
        {
            return;
        }
        consent.Ask(ConsentHost.Settings);
    }

    /// <summary>
    /// The model came (its download ended well) or went (removed): where each feature would send
    /// may have changed with no llm.choose, so the providers and the features' states are read again.
    /// </summary>
    public void Apply(InkEvent e)
    {
        var id = Row?.Id;
        if (id is null)
        {
            return;
        }
        var cameOrWent = e switch
        {
            ModelUpdateFinished { Ok: true } finished => finished.Next == id,
            ModelsListed { Ref: string reference } => reference == new CoreCommand.ModelRemove(id).CommandId,
            _ => false,
        };
        if (cameOrWent)
        {
            cloud.Load();
            ai.Polish.Load();
            ai.EditConsent.Load();
            ai.MeetingsConsent.Load();
        }
    }

    /// <summary>
    /// The caption under Settings > AI's language model: what this PC has of its own, and what
    /// Test and Try it send.
    /// </summary>
    public static string Caption(CloudModel cloud)
    {
        ArgumentNullException.ThrowIfNull(cloud);
        const string Tail = "Test sends the provider your key and one short fixed question, never your words. Nothing else is sent until you turn a feature on below and allow it.";
        if (cloud.OnDeviceInstalled)
        {
            return $"Polish, voice edit, summaries and Ask can run on this PC's own model, {cloud.OnDeviceName}: your words stay on this PC. Or pick a provider, paste your API key (kept in Windows Credential Manager, never in Inkwell's files) and press Use. {Tail}";
        }
        if (cloud.OnDevice is not null)
        {
            return $"This PC can download a language model of its own (On this PC, in the list above), or polish, voice edit, summaries and Ask can use one you bring: pick a provider, paste your API key (kept in Windows Credential Manager, never in Inkwell's files), choose a model and press Use. {Tail}";
        }
        return $"This PC has no language model of its own, so polish, voice edit, summaries and Ask use one you bring: pick a provider, paste your API key (kept in Windows Credential Manager, never in Inkwell's files), choose a model and press Use. {Tail}";
    }
}
