// The first run's models step as choices by outcome, not by model: what Inkwell can do with each,
// what it adds in the user's terms, then each model it takes on a line of its own (name, licence,
// size and host, the Mac's words), and one Download whose title carries the total of what is chosen. The first choice is the recommended set and is
// always taken; the others stay off until ticked, this PC's own language model (polish, voice edit
// and summaries without a key) among them. Nothing downloads until Download is pressed, and the
// downloads are the CatalogueModel's (they go on after the sheet). The free space on the models'
// volume shows under Download. Settings > Models keeps a row and a Download per model.
using System.Collections.Immutable;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>One outcome the models step offers, and the catalogue's models it takes.</summary>
/// <param name="Kind">Takes the catalogue's models of this kind instead of <paramref name="ModelIds"/> (the language model: one, whichever the core lists).</param>
public sealed record ModelChoice(string Id, string Title, ImmutableArray<string> ModelIds, bool Required, ModelKind? Kind = null);

/// <summary>Which optional choices are ticked. UI thread only, like every screen model.</summary>
public sealed class ModelChoices : ObservableModel
{
    public const string QwenId = "qwen3-asr-1.7b-q8";
    public const string DiarizerId = "nemotron-3-diarization-q8";

    /// <summary>The recommended set: dictation, live words and a meeting's transcript as it happens.</summary>
    public static ModelChoice Speech { get; } = new("speech", "Dictation, live words and meeting transcripts", CatalogueModel.RecommendedIds, Required: true);

    /// <summary>Qwen3-ASR: fewer mistakes, and on Windows a meeting's final pass.</summary>
    public static ModelChoice Accuracy { get; } = new("accuracy", "Fewer mistakes", [QwenId], Required: false);

    /// <summary>The diarizer: who said what on the far end.</summary>
    public static ModelChoice Speakers { get; } = new("speakers", "Tell the people on the call apart", [DiarizerId], Required: false);

    /// <summary>This PC's own language model: polish, voice edit and summaries with the words staying here. Unticked until the user ticks it.</summary>
    public static ModelChoice OnThisPc { get; } = new("on-this-pc", "Polish, edit and summaries on this PC", [], Required: false, Kind: ModelKind.Language);

    public static ImmutableArray<ModelChoice> All { get; } = [Speech, Accuracy, Speakers, OnThisPc];

    /// <summary>Under the Download button while it shows.</summary>
    public const string NothingUntilPressed = "Nothing downloads until you press Download.";

    private readonly HashSet<string> ticked = [];

    /// <summary>The choices this catalogue can give: those with at least one of their models listed.</summary>
    public static IReadOnlyList<ModelChoice> Shown(CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        return All.Where(c => Rows(c, catalogue).Count > 0).ToList();
    }

    /// <summary>Whether a choice shows ticked: always the required one; one installed or downloading; or one the user ticked.</summary>
    public bool IsTicked(ModelChoice choice, CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(choice);
        return choice.Required || ticked.Contains(choice.Id) || Taken(choice, catalogue);
    }

    /// <summary>Whether the user can tick or untick it: an optional choice not installed and not downloading.</summary>
    public static bool CanTick(ModelChoice choice, CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(choice);
        return !choice.Required && !Taken(choice, catalogue);
    }

    /// <summary>Ticks or unticks an optional choice. Nothing downloads.</summary>
    public void Tick(ModelChoice choice, bool on)
    {
        ArgumentNullException.ThrowIfNull(choice);
        if (choice.Required || (on ? !ticked.Add(choice.Id) : !ticked.Remove(choice.Id)))
        {
            return;
        }
        Changed();
    }

    /// <summary>
    /// Under a choice, one line per model it takes, smallest first (the order they download in):
    /// "Parakeet TDT v3 · CC-BY-4.0 · 639 MB · from huggingface.co" (sizes in Windows' units).
    /// </summary>
    public static IReadOnlyList<string> Lines(ModelChoice choice, CatalogueModel catalogue, IFormatProvider? format = null)
    {
        ArgumentNullException.ThrowIfNull(choice);
        return Rows(choice, catalogue)
            .OrderBy(r => r.Entry.SizeBytes)
            .Select(r => r.Line(format))
            .ToList();
    }

    /// <summary>
    /// What a choice adds over the set, in the user's terms (the Mac's words; on Windows the meeting
    /// model's final pass too); null for the set.
    /// </summary>
    public static string? Detail(ModelChoice choice)
    {
        ArgumentNullException.ThrowIfNull(choice);
        if (choice == Accuracy)
        {
            return "About a third fewer wrong words in dictation and meetings, and it gives meetings their final pass.";
        }
        if (choice == OnThisPc)
        {
            return "A language model of this PC's own, with no key or account: your words stay on this PC. Each feature asks before it is turned on.";
        }
        return choice == Speakers ? "Speaker 1, Speaker 2 instead of \u201CThem\u201D." : null;
    }

    /// <summary>
    /// Windows' honest line under the set while the meeting model is not in: Windows' Parakeet has
    /// no final pass for meetings. Null otherwise, and for the other choices.
    /// </summary>
    public static string? Note(ModelChoice choice, CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(choice);
        ArgumentNullException.ThrowIfNull(catalogue);
        return choice == Speech && catalogue.Rows.FirstOrDefault(r => r.Id == QwenId) is { Installed: false }
            ? "On Windows, a meeting keeps this live transcript until the meeting model, under Fewer mistakes, is added."
            : null;
    }

    /// <summary>Whether every model a choice takes is on this PC.</summary>
    public static bool Installed(ModelChoice choice, CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(choice);
        var rows = Rows(choice, catalogue);
        return rows.Count > 0 && rows.All(r => r.Installed);
    }

    /// <summary>Where a choice's download is: installed, downloading, waiting, failed; null before any.</summary>
    public static string? Status(ModelChoice choice, CatalogueModel catalogue, IFormatProvider? format = null)
    {
        if (Installed(choice, catalogue))
        {
            return "Installed";
        }
        var rows = Rows(choice, catalogue);
        if (rows.FirstOrDefault(r => r.CanRetry) is { } failed)
        {
            return failed.Status(format);
        }
        if (rows.FirstOrDefault(r => r.Download is ModelDownload.Running) is { } running)
        {
            return $"{running.Name}: {running.Status(format)}";
        }
        return rows.Any(r => r.Download is ModelDownload.Waiting) ? "Waiting for the download before it" : null;
    }

    /// <summary>The models Download would ask for: those of the ticked choices not installed and not asked for (or failed, to retry).</summary>
    public IReadOnlyList<CatalogueEntry> ToDownload(CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        return Shown(catalogue)
            .Where(c => IsTicked(c, catalogue))
            .SelectMany(c => Rows(c, catalogue))
            .Where(r => r.CanDownload || r.CanRetry)
            .Select(r => r.Entry)
            .ToList();
    }

    /// <summary>The Download button: "Download 640 MB", the total of what is chosen; null when there is nothing to download.</summary>
    public string? DownloadTitle(CatalogueModel catalogue, IFormatProvider? format = null)
    {
        var models = ToDownload(catalogue);
        return models.Count == 0 ? null : $"Download {StorageModel.Size(models.Sum(m => m.SizeBytes), format)}";
    }

    /// <summary>Under Download: the free space on the volume models go on ("47.2 GB free on C:"); null while it is not known.</summary>
    public static string? FreeSpaceLine(CatalogueModel catalogue, IFormatProvider? format = null)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        return catalogue.FreeSpaceText(format);
    }

    /// <summary>The Download button for screen readers: which models, how much in all, from where.</summary>
    public string DownloadName(CatalogueModel catalogue, IFormatProvider? format = null)
    {
        var models = ToDownload(catalogue);
        return $"Download {And(models.Select(m => CatalogueModel.Name(m)))}: {StorageModel.Size(models.Sum(m => m.SizeBytes), format)} in all, from {And(models.Select(m => CatalogueModel.Source(m.Id)).Distinct())}";
    }

    /// <summary>Downloads what is chosen, smallest first (the step's Download).</summary>
    public void Download(CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        catalogue.Download(ToDownload(catalogue).Select(m => m.Id));
    }

    /// <summary>Installed, or asked for: the choice is taken and its tick stays.</summary>
    private static bool Taken(ModelChoice choice, CatalogueModel catalogue)
    {
        var rows = Rows(choice, catalogue);
        // A failed, refused or cancelled download is not taken: its box can be unticked.
        return rows.Count > 0 && rows.All(r => r.Installed || r.Download is ModelDownload.Waiting or ModelDownload.Running);
    }

    private static List<ModelRow> Rows(ModelChoice choice, CatalogueModel catalogue)
    {
        ArgumentNullException.ThrowIfNull(catalogue);
        var rows = catalogue.Rows;
        if (choice.Kind is ModelKind kind)
        {
            return rows.Where(r => r.Entry.Kind == kind).ToList();
        }
        return choice.ModelIds.Select(id => rows.FirstOrDefault(r => r.Id == id)).OfType<ModelRow>().ToList();
    }

    /// <summary>"a", "a and b", "a, b and c".</summary>
    private static string And(IEnumerable<string> items)
    {
        var list = items.ToList();
        return list.Count < 2 ? string.Concat(list) : $"{string.Join(", ", list.Take(list.Count - 1))} and {list[^1]}";
    }
}
