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
}

/// <summary>A catalogue model's row where it can be downloaded (Settings > Models and the first run's models step).</summary>
/// <param name="Download">Its download this run; null when it was not asked for.</param>
public sealed record ModelRow(CatalogueEntry Entry, ModelDownload? Download)
{
    public string Id => Entry.Id;

    public string Name => CatalogueModel.Name(Entry.Id);

    /// <summary>Installed: as the catalogue listed it, or by a download this run (before the list is read again).</summary>
    public bool Installed => Entry.Installed || Download is ModelDownload.Installed;

    /// <summary>Not installed and not asked for: its Download shows.</summary>
    public bool CanDownload => !Installed && Download is null;

    /// <summary>Its download failed: Retry shows.</summary>
    public bool CanRetry => Download is ModelDownload.Failed;

    /// <summary>Its line: name, licence, size, and whether it is installed.</summary>
    public string Text(IFormatProvider? format = null) => CatalogueModel.Downloadable(Entry with { Installed = Installed }, format);

    /// <summary>Where it would come from, while it can be downloaded.</summary>
    public string? From => CanDownload ? $"From {CatalogueModel.Source(Id)}" : null;

    /// <summary>What its download is doing, or why it failed; null when there is nothing to say.</summary>
    public string? Status(IFormatProvider? format = null) => Download switch
    {
        ModelDownload.Waiting => "Waiting for the download before it",
        ModelDownload.Running { DoneBytes: <= 0 } => "Starting the download…",
        ModelDownload.Running running => $"{StorageModel.Size(running.DoneBytes, format)} of {StorageModel.Size(running.TotalBytes, format)}",
        ModelDownload.Failed failed => $"Couldn't download {Name}: {failed.Message}",
        _ => null,
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

    /// <summary>The catalogue's models for this OS.</summary>
    public IReadOnlyList<CatalogueEntry> Models { get; private set; } = [];

    /// <summary>A models.listed has arrived: the list is known (it may be empty).</summary>
    public bool Listed { get; private set; }

    /// <summary>The last models.list failed: the list is not known, which is not the same as empty.</summary>
    public bool Failed { get; private set; }

    /// <summary>The catalogue's models with their downloads, in the catalogue's order.</summary>
    public IReadOnlyList<ModelRow> Rows => Models.Select(m => new ModelRow(m, downloads.GetValueOrDefault(m.Id))).ToList();

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
    /// Downloads every model not installed and not asked for yet, smallest first (the first run's
    /// Download), as the Mac's does: voice detection and Parakeet (live words, and dictation on a
    /// PC without a GPU) are in long before Qwen3-ASR's gigabytes.
    /// </summary>
    public void DownloadMissing()
    {
        var asked = false;
        foreach (var entry in NotAskedFor.OrderBy(m => m.SizeBytes))
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

    /// <summary>Downloads the recommended set's missing models, smallest first (the first run's Download, Today's).</summary>
    public void DownloadRecommended()
    {
        var asked = false;
        foreach (var entry in RecommendedNotAskedFor.OrderBy(m => m.SizeBytes))
        {
            asked |= Ask(entry.Id);
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
        return true;
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
        running = null;
        Pump();
        Changed();
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

    /// <summary>A downloadable model's line: name, licence, size, and whether it is installed.</summary>
    public static string Downloadable(CatalogueEntry entry, IFormatProvider? format = null)
    {
        ArgumentNullException.ThrowIfNull(entry);
        return $"{Name(entry.Id)} · {entry.Licence} · {StorageModel.Size(entry.SizeBytes, format)} · {(entry.Installed ? "installed" : "not installed")}";
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
        return failed.Command is "models.list" or "engine.route" or "model.update";
    }

    /// <summary>The job an engine.route failure is about, from its id; null when it names none.</summary>
    private static Job? RouteJob(CommandFailed failed) =>
        Jobs.Cast<Job?>().FirstOrDefault(j => failed.Id == new CoreCommand.EngineRoute(j!.Value).CommandId);

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case ModelsListed listed:
                Models = listed.Models;
                Listed = true;
                Failed = false;
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
                    Ended(finished.Ok ? new ModelDownload.Installed() : new ModelDownload.Failed(finished.Message ?? NoReasonText));
                }
                break;
            case CommandFailed failed when failed.Command == "model.update" && running is string id && failed.Id == new CoreCommand.ModelUpdate(id, id).CommandId:
                // Refused before it started (another update holds the model, or it never reached the core).
                Ended(new ModelDownload.Failed(failed.Message));
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
                Changed();
                break;
            default:
                break;
        }
    }
}
