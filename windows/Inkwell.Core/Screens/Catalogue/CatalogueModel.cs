// Settings > Models: which engine does each job now, and how accurate it measured. As the Mac's
// CatalogueModel. It also downloads the catalogue's models, for this screen and the first run's
// models step.
//
// What serves a job is the router's answer to engine.route, and engine.routed answers only that
// command: nothing announces a change. So the screen asks again whenever the answer may have
// changed: when it appears, after a model update finishes (a model installed or replaced), and
// when an engine the shell registers comes or goes. Windows has no Apple engines: the ids the
// screen names are the core's registry models and whatever the Windows shell registers (S3.2).
//
// Downloads: nothing downloads until the user presses a Download button that says what, how much
// and from where (Download, DownloadMissing). Each is a model.update with model and next the same
// registry id (the core installs it and loads nothing), one at a time in the order asked for: the
// next is sent only once the one before has ended, by model.update_finished or as a failed
// command. model.update_progress moves its row; a failure shows on its row in the core's words,
// and Retry asks again (the core resumes what it has on disk). A model that does dictation is kept
// warm once its download ends well (model.warm, as the Mac's), sent before the next download so it
// never waits for that one.
//
// Every model also has Cancel while it downloads or waits (model.cancel for the one the core runs;
// one still waiting here is only taken off the list) and Remove once it is installed (model.remove,
// only after the user confirmed it). A cancelled download keeps its part files, so Download picks
// up where it stopped. A refusal says why on the row: not enough room (the core's needed_bytes and
// free_bytes, nothing fetched) or the model in use (nothing deleted). The list carries the free
// space on the volume models go on, and the language model (Windows' polish, voice edit and
// summaries on this PC) is a row like the speech models.
using System.Collections.Immutable;
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>One job's line.</summary>
/// <param name="Engine">The engine's name; null when nothing fills the job.</param>
/// <param name="Wer">Its measured word error rate on this job, percent.</param>
/// <param name="Known">Whether the answer has arrived.</param>
/// <param name="Failed">The last question about it failed (engine.route): not known, which is not "nothing installed".</param>
public sealed record CatalogueLine(Job Job, string? Engine, double? Wer, bool Known, bool Failed = false)
{
    public const string FailedText = "Couldn't check which model does this";

    /// <summary>The measured accuracy, as the screen shows it.</summary>
    public string? Accuracy => Wer is double wer
        ? string.Format(CultureInfo.InvariantCulture, "{0:0.0} % of words right ({1:0.0} % word error rate)", Math.Max(0, 100 - wer), wer)
        : null;

    /// <summary>What the line says in place of an engine: the engine, else nothing or still asking.</summary>
    public string EngineText => Engine ?? (Failed ? FailedText : Known ? "Nothing installed yet" : "Checking…");
}

/// <summary>Where a model's download is, this run.</summary>
public abstract record ModelDownload
{
    private ModelDownload() { }

    /// <summary>Asked for: it waits for the download before it (one runs at a time).</summary>
    public sealed record Waiting : ModelDownload;

    /// <summary>Downloading: bytes on disk so far, of its size.</summary>
    public sealed record Running(long DoneBytes, long TotalBytes) : ModelDownload;

    /// <summary>Its files are installed and checked.</summary>
    public sealed record Installed : ModelDownload;

    /// <summary>It stopped, and why (the core's words).</summary>
    public sealed record Failed(string Message) : ModelDownload;

    /// <summary>The core refused it before fetching anything: the volume has too little room.</summary>
    /// <param name="NeededBytes">What must be free: what the download still needs, plus the core's margin.</param>
    /// <param name="FreeBytes">What is free now.</param>
    public sealed record NoSpace(long NeededBytes, long FreeBytes) : ModelDownload;

    /// <summary>The user cancelled it: its part files are kept, and Download resumes them.</summary>
    public sealed record Cancelled : ModelDownload;
}

/// <summary>A catalogue model's row where it can be downloaded (Settings > Models and the first run's models step).</summary>
/// <param name="Download">Its download this run; null when it was not asked for.</param>
public sealed record ModelRow(CatalogueEntry Entry, ModelDownload? Download)
{
    public string Id => Entry.Id;

    public string Name => CatalogueModel.Name(Entry);

    /// <summary>The language model (polish, voice edit and summaries on this PC), not a speech model.</summary>
    public bool IsLanguage => Entry.Kind == ModelKind.Language;

    /// <summary>Its Cancel was sent and the core has not ended the download yet.</summary>
    public bool Cancelling { get; init; }

    /// <summary>Its Remove was sent and the core has not answered yet.</summary>
    public bool Removing { get; init; }

    /// <summary>Why its last Cancel or Remove did nothing, in words; null when there is nothing to say.</summary>
    public string? Note { get; init; }

    /// <summary>The volume models go on ("C:"), for what a refusal for room says; null when not known.</summary>
    public string? Volume { get; init; }

    /// <summary>Installed: as the catalogue listed it, or by a download this run (before the list is read again).</summary>
    public bool Installed => Entry.Installed || Download is ModelDownload.Installed;

    /// <summary>Not installed and not asked for (or cancelled): its Download shows.</summary>
    public bool CanDownload => !Installed && Download is null or ModelDownload.Cancelled;

    /// <summary>Its download failed, or was refused for room: Retry shows.</summary>
    public bool CanRetry => Download is ModelDownload.Failed or ModelDownload.NoSpace;

    /// <summary>It downloads or waits to, and no Cancel is on its way: Cancel shows.</summary>
    public bool CanCancel => Download is ModelDownload.Waiting or ModelDownload.Running && !Cancelling;

    /// <summary>It is installed, no download of it runs, and no Remove is on its way: Remove shows.</summary>
    public bool CanRemove => Installed && Download is not (ModelDownload.Waiting or ModelDownload.Running) && !Removing;

    /// <summary>Its line: name, licence, size, and whether it is installed.</summary>
    public string Text(IFormatProvider? format = null) => CatalogueModel.Downloadable(Entry with { Installed = Installed }, format);

    /// <summary>
    /// The first run's line and Settings > AI's: "Qwen3 4B Instruct · Apache-2.0 · 2.32 GB · from
    /// huggingface.co" (sizes in Windows' units).
    /// </summary>
    public string Line(IFormatProvider? format = null) =>
        $"{Name} · {Entry.Licence} · {StorageModel.Size(Entry.SizeBytes, format)} · from {CatalogueModel.Source(Id)}";

    /// <summary>Where it would come from, while it can be downloaded.</summary>
    public string? From => CanDownload ? $"From {CatalogueModel.Source(Id)}" : null;

    /// <summary>What its download is doing, or why it failed; null when there is nothing to say.</summary>
    public string? Status(IFormatProvider? format = null) => Download switch
    {
        ModelDownload.Waiting when Cancelling => "Stopping…",
        ModelDownload.Running when Cancelling => "Stopping the download…",
        ModelDownload.Waiting => "Waiting for the download before it",
        ModelDownload.Running { DoneBytes: <= 0 } => "Starting the download…",
        ModelDownload.Running running => $"{StorageModel.Size(running.DoneBytes, format)} of {StorageModel.Size(running.TotalBytes, format)}",
        ModelDownload.Failed failed => $"Couldn't download {Name}: {failed.Message}",
        ModelDownload.NoSpace room => CatalogueModel.NoSpaceText(room.NeededBytes, room.FreeBytes, Volume, format),
        ModelDownload.Cancelled => "Stopped. Download picks up where it left off.",
        _ => Removing ? "Removing…" : null,
    };

    /// <summary>How far its download has got, 0 to 1, while it runs; null otherwise.</summary>
    public double? Progress => Download is ModelDownload.Running running
        ? running.TotalBytes > 0 ? Math.Clamp((double)running.DoneBytes / running.TotalBytes, 0, 1) : 0
        : null;

    /// <summary>Its Download button's name for screen readers: what, how much, from where.</summary>
    public string DownloadName(IFormatProvider? format = null) =>
        $"Download {Name}, {StorageModel.Size(Entry.SizeBytes, format)}, from {CatalogueModel.Source(Id)}";

    public string RetryName => $"Retry downloading {Name}";

    public string ProgressName => $"Downloading {Name}";

    public string CancelName => $"Cancel downloading {Name}";

    public string RemoveName => $"Remove {Name} from this PC";

    /// <summary>What Remove asks before it deletes anything.</summary>
    public string RemoveQuestion => $"Remove {Name} from this PC?";

    /// <summary>What Remove deletes, said with the question.</summary>
    public string RemoveDetail(IFormatProvider? format = null) => IsLanguage
        ? $"Its files ({StorageModel.Size(Entry.SizeBytes, format)}) are deleted. Polish, voice edit and summaries stop using it until it is downloaded again, and nothing else takes its place by itself."
        : $"Its files ({StorageModel.Size(Entry.SizeBytes, format)}) are deleted. What it does stops until it is downloaded again.";

    /// <summary>The question's button that deletes.</summary>
    public const string RemoveConfirm = "Remove";
}

public sealed class CatalogueModel(Action<CoreCommand> send) : ObservableModel
{
    /// <summary>The jobs the screen lists, in order.</summary>
    public static ImmutableArray<Job> Jobs { get; } = [Job.DictationFinal, Job.MeetingFinal, Job.LivePartials];

    public const string FailedText = "The model list could not be read.";

    /// <summary>The line under the list.</summary>
    public const string SourceText = "Accuracy is measured on public test sets: AMI meetings and FLEURS English.";

    /// <summary>A download's failure when the core gave no reason.</summary>
    public const string NoReasonText = "the core gave no reason";

    /// <summary>A download's failure when the core stopped while it ran or waited.</summary>
    public const string CoreStoppedText = "the core stopped";

    private ImmutableDictionary<string, IReadOnlyList<JobScore>> shellEngines = ImmutableDictionary<string, IReadOnlyList<JobScore>>.Empty;
    private ImmutableDictionary<Job, EngineRouted> serving = ImmutableDictionary<Job, EngineRouted>.Empty;
    private ImmutableHashSet<Job> routeFailed = [];
    private ImmutableDictionary<string, ModelDownload> downloads = ImmutableDictionary<string, ModelDownload>.Empty;
    /// <summary>Downloads asked for, in order, behind the one running.</summary>
    private ImmutableList<string> waiting = [];
    /// <summary>The model whose model.update was sent and has not ended.</summary>
    private string? running;
    /// <summary>Models whose model.cancel was sent, until their download ends.</summary>
    private ImmutableHashSet<string> cancelling = [];
    /// <summary>Models whose model.remove was sent, until the core answers.</summary>
    private ImmutableHashSet<string> removing = [];
    /// <summary>Why a row's last Cancel or Remove did nothing, by model.</summary>
    private ImmutableDictionary<string, string> notes = ImmutableDictionary<string, string>.Empty;

    /// <summary>The catalogue's models for this OS.</summary>
    public IReadOnlyList<CatalogueEntry> Models { get; private set; } = [];

    /// <summary>A models.listed has arrived: the list is known (it may be empty).</summary>
    public bool Listed { get; private set; }

    /// <summary>The last models.list failed: the list is not known, which is not the same as empty.</summary>
    public bool Failed { get; private set; }

    /// <summary>The bytes free on the volume models go on, from the last list; null when the OS could not say.</summary>
    public long? FreeBytes { get; private set; }

    /// <summary>
    /// The volume models go on, as Windows names it ("C:"): what the free-space line and a refusal
    /// for room say. Null when it is not a drive letter (the lines then name no volume).
    /// </summary>
    public string? Volume { get; set; }

    /// <summary>The drive letter of <paramref name="folder"/> ("C:"), or null for a folder that has none (a share, or none known).</summary>
    public static string? VolumeOf(string? folder) =>
        folder is { Length: >= 2 } f && char.IsAsciiLetter(f[0]) && f[1] == ':' ? $"{char.ToUpperInvariant(f[0])}:" : null;

    /// <summary>"47.2 GB free on C:", under the models; null while the free space is not known.</summary>
    public string? FreeSpaceText(IFormatProvider? format = null) => FreeBytes is long free
        ? Volume is string volume ? $"{StorageModel.Size(free, format)} free on {volume}" : $"{StorageModel.Size(free, format)} free for models"
        : null;

    /// <summary>A download refused for room: "Needs 3.40 GB; 1.20 GB free on C:".</summary>
    public static string NoSpaceText(long needed, long free, string? volume, IFormatProvider? format = null) =>
        $"Needs {StorageModel.Size(needed, format)}; {StorageModel.Size(free, format)} free{(volume is null ? "" : $" on {volume}")}";

    /// <summary>The catalogue's models with their downloads, in the catalogue's order.</summary>
    public IReadOnlyList<ModelRow> Rows => Models.Select(m => new ModelRow(m, downloads.GetValueOrDefault(m.Id))
    {
        Cancelling = cancelling.Contains(m.Id),
        Removing = removing.Contains(m.Id),
        Note = notes.GetValueOrDefault(m.Id),
        Volume = Volume,
    }).ToList();

    /// <summary>The language model's row (Windows' own model for polish, voice edit and summaries); null when the catalogue has none.</summary>
    public ModelRow? LanguageRow => Rows.FirstOrDefault(r => r.IsLanguage);

    /// <summary>The models not installed and not asked for yet: what the first run's Download fetches.</summary>
    public IReadOnlyList<CatalogueEntry> NotAskedFor => Rows.Where(r => r.CanDownload).Select(r => r.Entry).ToList();

    /// <summary>A download runs (others may wait behind it).</summary>
    public bool Downloading => running is not null;

    /// <summary>Times the screen asked again (tests and diagnostics).</summary>
    public int Requeries { get; private set; }

    /// <summary>Asks what serves every job, and for the catalogue.</summary>
    public void Requery()
    {
        Requeries++;
        send(new CoreCommand.ModelsList());
        foreach (var job in Jobs)
        {
            send(new CoreCommand.EngineRoute(job));
        }
    }

    /// <summary>
    /// Downloads a model (its row's Download or Retry), after the downloads asked for before it.
    /// Nothing for a model that is installed, already asked for, or not in the catalogue.
    /// </summary>
    public void Download(string id)
    {
        ArgumentNullException.ThrowIfNull(id);
        if (Ask(id))
        {
            Pump();
            Changed();
        }
    }

    /// <summary>
    /// Downloads every speech model not installed and not asked for yet, smallest first, as the
    /// Mac's does: voice detection and Parakeet (live words, and dictation on a PC without a GPU)
    /// are in long before Qwen3-ASR's gigabytes. Never the language model: that is the user's own
    /// choice (the first run's, or its row's Download).
    /// </summary>
    public void DownloadMissing()
    {
        var asked = false;
        foreach (var entry in NotAskedFor.Where(m => m.Kind != ModelKind.Language).OrderBy(m => m.SizeBytes))
        {
            asked |= Ask(entry.Id);
        }
        if (asked)
        {
            Pump();
            Changed();
        }
    }

    /// <summary>
    /// Windows' recommended set: Silero VAD (voice detection) and Windows' Parakeet TDT v3 (live
    /// words and dictation), about 640 MB. Qwen3-ASR (the meeting final pass, about 2.5 GB) and
    /// the diarizer are optional.
    /// </summary>
    public static ImmutableArray<string> RecommendedIds { get; } = ["silero-vad-v6-16k", "parakeet-tdt-0.6b-v3-int8"];

    /// <summary>Whether <paramref name="id"/> is in the recommended set.</summary>
    public static bool IsRecommended(string id) => RecommendedIds.Contains(id);

    /// <summary>The recommended models not installed and not asked for yet.</summary>
    public IReadOnlyList<CatalogueEntry> RecommendedNotAskedFor => NotAskedFor.Where(m => IsRecommended(m.Id)).ToList();

    /// <summary>Downloads the recommended set's missing models, smallest first (Today's Download).</summary>
    public void DownloadRecommended() => Download(RecommendedIds);

    /// <summary>
    /// Downloads these models, smallest first, after the downloads asked for before them (the first
    /// run's Download: ModelChoices). A model installed, already asked for or not listed is passed
    /// over; one that failed is retried.
    /// </summary>
    public void Download(IEnumerable<string> ids)
    {
        ArgumentNullException.ThrowIfNull(ids);
        var asked = false;
        foreach (var row in Rows.Where(r => ids.Contains(r.Id)).OrderBy(r => r.Entry.SizeBytes))
        {
            asked |= Ask(row.Id);
        }
        if (asked)
        {
            Pump();
            Changed();
        }
    }

    /// <summary>Queues a model that can be downloaded, or retried; false for any other.</summary>
    private bool Ask(string id)
    {
        if (Rows.FirstOrDefault(r => r.Id == id) is not { } row || !(row.CanDownload || row.CanRetry))
        {
            return false;
        }
        downloads = downloads.SetItem(id, new ModelDownload.Waiting());
        waiting = waiting.Add(id);
        notes = notes.Remove(id);
        return true;
    }

    /// <summary>
    /// A row's Cancel: one still waiting here is taken off the list (nothing was sent for it); the
    /// one the core runs gets model.cancel, and its row says it is stopping until the update ends.
    /// Nothing for a model that is not downloading.
    /// </summary>
    public void Cancel(string id)
    {
        ArgumentNullException.ThrowIfNull(id);
        if (Rows.FirstOrDefault(r => r.Id == id) is not { CanCancel: true })
        {
            return;
        }
        notes = notes.Remove(id);
        if (running == id)
        {
            cancelling = cancelling.Add(id);
            send(new CoreCommand.ModelCancel(id));
        }
        else
        {
            waiting = waiting.Remove(id);
            downloads = downloads.Remove(id);
        }
        Changed();
    }

    /// <summary>
    /// A row's Remove, once the user confirmed it: model.remove, and the row says it is removing
    /// until the core answers with the list (or refuses: the model in use). Nothing for a model
    /// that is not installed, or downloads.
    /// </summary>
    public void Remove(string id)
    {
        ArgumentNullException.ThrowIfNull(id);
        if (Rows.FirstOrDefault(r => r.Id == id) is not { CanRemove: true })
        {
            return;
        }
        notes = notes.Remove(id);
        removing = removing.Add(id);
        send(new CoreCommand.ModelRemove(id));
        Changed();
    }

    /// <summary>Sends the next download when none runs: model == next, the first download of that model.</summary>
    private void Pump()
    {
        if (running is not null || waiting.IsEmpty)
        {
            return;
        }
        var id = waiting[0];
        waiting = waiting.RemoveAt(0);
        running = id;
        downloads = downloads.SetItem(id, new ModelDownload.Running(0, Models.FirstOrDefault(m => m.Id == id)?.SizeBytes ?? 0));
        send(new CoreCommand.ModelUpdate(id, id));
    }

    /// <summary>Whether the catalogue lists the model doing dictation (Silero VAD and Nemotron do not).</summary>
    private bool Dictates(string id) =>
        Models.FirstOrDefault(m => m.Id == id)?.Jobs.Any(j => j.Job == Job.DictationFinal) == true;

    /// <summary>The running download ended; the next one goes.</summary>
    private void Ended(ModelDownload outcome)
    {
        downloads = downloads.SetItem(running!, outcome);
        cancelling = cancelling.Remove(running!);
        running = null;
        Pump();
        Changed();
    }

    /// <summary>
    /// Whether a speech model is installed: any of dictation, meeting transcript and live words
    /// served by a model. Null until all three have answered (or one could not be asked): never
    /// "no model" on a guess.
    /// </summary>
    public bool? HasSpeechModel
    {
        get
        {
            var lines = Jobs.Select(Line).ToList();
            if (lines.Any(l => l.Engine is not null))
            {
                return true;
            }
            return lines.All(l => l.Known) ? false : null;
        }
    }

    public CatalogueLine Line(Job job)
    {
        if (routeFailed.Contains(job))
        {
            return new CatalogueLine(job, null, null, Known: false, Failed: true);
        }
        if (!serving.TryGetValue(job, out var routed))
        {
            return new CatalogueLine(job, null, null, Known: false);
        }
        if (routed.Id is not string id)
        {
            return new CatalogueLine(job, null, null, Known: true);
        }
        var scores = routed.Source == EngineSource.Shell
            ? shellEngines.GetValueOrDefault(id)
            : Models.FirstOrDefault(m => m.Id == id)?.Jobs;
        var wer = scores?.FirstOrDefault(s => s.Job == job)?.Wer;
        return new CatalogueLine(job, Name(id), wer, Known: true);
    }

    /// <summary>A job's name.</summary>
    public static string Title(Job job) => job switch
    {
        Job.DictationFinal => "Dictation",
        Job.MeetingFinal => "Meeting transcript",
        Job.LivePartials => "Live words",
        Job.Diarization => "Who spoke",
        Job.VoiceActivity => "Voice detection",
        _ => job.ToString(),
    };

    /// <summary>
    /// An engine's name. Ids this build does not know are shown as they are: they name a model,
    /// not an app.
    /// </summary>
    public static string Name(string id) => id switch
    {
        "qwen3-asr-1.7b-q8" => "Qwen3-ASR 1.7B",
        "fluidaudio-parakeet-tdt-0.6b-v3" or "fluidaudio-parakeet-tdt-0.6b-v3-offline" or "parakeet-tdt-0.6b-v3-int8" => "Parakeet TDT v3",
        "nemotron-3-diarization-q8" => "Nemotron-3-Diarization",
        "silero-vad-v6-16k" => "Silero VAD",
        _ => id,
    };

    /// <summary>A catalogue model's name: a language model's as the core names it, a speech model's by its id.</summary>
    public static string Name(CatalogueEntry entry)
    {
        ArgumentNullException.ThrowIfNull(entry);
        return entry.Kind == ModelKind.Language && !string.IsNullOrWhiteSpace(entry.Name) ? entry.Name : Name(entry.Id);
    }

    /// <summary>A downloadable model's line: name, licence, size, and whether it is installed.</summary>
    public static string Downloadable(CatalogueEntry entry, IFormatProvider? format = null)
    {
        ArgumentNullException.ThrowIfNull(entry);
        return $"{Name(entry)} · {entry.Licence} · {StorageModel.Size(entry.SizeBytes, format)} · {(entry.Installed ? "installed" : "not installed")}";
    }

    /// <summary>
    /// Where a model's files come from, as its Download says before the user agrees: the host of
    /// the URLs its row names in the core's registry (ink-engines), as the Mac says it. Every row
    /// is on huggingface.co except Silero VAD's, on raw.githubusercontent.com.
    /// </summary>
    public static string Source(string id) => id == "silero-vad-v6-16k" ? "raw.githubusercontent.com" : "huggingface.co";

    /// <summary>
    /// Whether this screen shows the failure: models.list, engine.route on its job's line, and
    /// model.update on its model's row.
    /// </summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command is "models.list" or "engine.route" or "model.update" or "model.cancel" or "model.remove";
    }

    /// <summary>The model a model.cancel or model.remove failure is about, from its id; null when it names none.</summary>
    private static string? RowOf(CommandFailed failed)
    {
        var prefix = $"{failed.Command}:";
        return failed.Id is string id && id.StartsWith(prefix, StringComparison.Ordinal) ? id[prefix.Length..] : null;
    }

    /// <summary>The job an engine.route failure is about, from its id; null when it names none.</summary>
    private static Job? RouteJob(CommandFailed failed) =>
        Jobs.Cast<Job?>().FirstOrDefault(j => failed.Id == new CoreCommand.EngineRoute(j!.Value).CommandId);

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case ModelsListed listed:
                // Speech and language models alike (a list from a core before language models has
                // no kind: speech).
                Models = listed.Models;
                FreeBytes = listed.FreeBytes;
                Listed = true;
                Failed = false;
                if (listed.Ref is string reference && removing.FirstOrDefault(m => new CoreCommand.ModelRemove(m).CommandId == reference) is string removed)
                {
                    // Removed: the list says so; a download of it this run is history.
                    removing = removing.Remove(removed);
                    downloads = downloads.Remove(removed);
                    // What serves each job may have changed with it.
                    foreach (var job in Jobs)
                    {
                        send(new CoreCommand.EngineRoute(job));
                    }
                }
                Changed();
                break;
            case CommandFailed failed when failed.Command == "models.list":
                Failed = true;
                Changed();
                break;
            case EngineRouted routed:
                serving = serving.SetItem(routed.Job, routed);
                routeFailed = routeFailed.Remove(routed.Job);
                Changed();
                break;
            case CommandFailed failed when failed.Command == "engine.route" && RouteJob(failed) is Job job:
                routeFailed = routeFailed.Add(job);
                Changed();
                break;
            case EngineRegistered engine:
                shellEngines = shellEngines.SetItem(engine.Id, engine.Jobs);
                Changed();
                Requery();
                break;
            case EngineUnregistered engine:
                shellEngines = shellEngines.Remove(engine.Id);
                Changed();
                Requery();
                break;
            case ModelUpdateProgress progress when progress.Next == running:
                downloads = downloads.SetItem(progress.Next, new ModelDownload.Running(progress.DoneBytes, progress.TotalBytes));
                Changed();
                break;
            case ModelUpdateFinished finished:
                // Asked before the next download is sent: engine.route waits behind a model.update.
                Requery();
                if (finished.Next == running)
                {
                    // Warmed now, not by the first dictation after the download.
                    if (finished.Ok && Dictates(finished.Next))
                    {
                        send(new CoreCommand.ModelWarm(Job.DictationFinal));
                    }
                    Ended(finished.Ok ? new ModelDownload.Installed()
                        : finished.Cancelled == true ? new ModelDownload.Cancelled()
                        : new ModelDownload.Failed(finished.Message ?? NoReasonText));
                }
                break;
            case CommandFailed failed when failed.Command == "model.update" && running is string id && failed.Id == new CoreCommand.ModelUpdate(id, id).CommandId:
                // Refused before it started: no room (nothing fetched), another update holds the
                // model, or it never reached the core.
                Ended(failed is { Code: FailureCode.NotEnoughSpace, NeededBytes: long needed, FreeBytes: long free }
                    ? new ModelDownload.NoSpace(needed, free)
                    : new ModelDownload.Failed(failed.Message));
                break;
            case CommandFailed failed when failed.Command == "model.cancel" && RowOf(failed) is string cancelled:
                // No download of it was running (it ended just before): its own end says how. Any
                // other refusal leaves it running, and the row says so.
                cancelling = cancelling.Remove(cancelled);
                if (failed.Code != FailureCode.NotDownloading)
                {
                    notes = notes.SetItem(cancelled, $"Couldn't stop the download: {failed.Message}");
                }
                Changed();
                break;
            case CommandFailed failed when failed.Command == "model.remove" && RowOf(failed) is string kept && removing.Contains(kept):
                removing = removing.Remove(kept);
                var name = Models.FirstOrDefault(m => m.Id == kept) is { } entry ? Name(entry) : Name(kept);
                notes = notes.SetItem(kept, failed.Code == FailureCode.ModelInUse
                    ? $"{name} is in use, so it wasn't removed. Try again once it's done."
                    : $"Couldn't remove {name}: {failed.Message}");
                Changed();
                break;
            case CoreStopped:
                shellEngines = shellEngines.Clear();
                serving = serving.Clear();
                routeFailed = routeFailed.Clear();
                // A download running or waiting ends with the core: its row says so, with Retry.
                foreach (var stopped in running is null ? waiting : waiting.Insert(0, running))
                {
                    downloads = downloads.SetItem(stopped, new ModelDownload.Failed(CoreStoppedText));
                }
                waiting = waiting.Clear();
                running = null;
                cancelling = cancelling.Clear();
                // A removal the core never answered: the next list says whether it happened.
                removing = removing.Clear();
                Changed();
                break;
            default:
                break;
        }
    }
}
