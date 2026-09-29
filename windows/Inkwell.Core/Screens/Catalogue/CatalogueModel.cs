// Settings > Models: which engine does each job now, and how accurate it measured. Read-only. As
// the Mac's CatalogueModel.
//
// What serves a job is the router's answer to engine.route, and engine.routed answers only that
// command: nothing announces a change. So the screen asks again whenever the answer may have
// changed: when it appears, after a model update finishes (a model installed or replaced), and
// when an engine the shell registers comes or goes. Windows has no Apple engines: the ids the
// screen names are the core's registry models and whatever the Windows shell registers (S3.2).
using System.Collections.Immutable;
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>One job's line.</summary>
/// <param name="Engine">The engine's name; null when nothing fills the job.</param>
/// <param name="Wer">Its measured word error rate on this job, percent.</param>
/// <param name="Known">Whether the answer has arrived.</param>
public sealed record CatalogueLine(Job Job, string? Engine, double? Wer, bool Known)
{
    /// <summary>The measured accuracy, as the screen shows it.</summary>
    public string? Accuracy => Wer is double wer
        ? string.Format(CultureInfo.InvariantCulture, "{0:0.0} % of words right ({1:0.0} % word error rate)", Math.Max(0, 100 - wer), wer)
        : null;

    /// <summary>What the line says in place of an engine: the engine, else nothing or still asking.</summary>
    public string EngineText => Engine ?? (Known ? "Nothing installed yet" : "Checking…");
}

public sealed class CatalogueModel(Action<CoreCommand> send) : ObservableModel
{
    /// <summary>The jobs the screen lists, in order.</summary>
    public static ImmutableArray<Job> Jobs { get; } = [Job.DictationFinal, Job.MeetingFinal, Job.LivePartials];

    public const string FailedText = "The model list could not be read.";

    /// <summary>The line under the list.</summary>
    public const string SourceText = "Accuracy is measured on public test sets: AMI meetings and FLEURS English.";

    private ImmutableDictionary<string, IReadOnlyList<JobScore>> shellEngines = ImmutableDictionary<string, IReadOnlyList<JobScore>>.Empty;
    private ImmutableDictionary<Job, EngineRouted> serving = ImmutableDictionary<Job, EngineRouted>.Empty;

    /// <summary>The catalogue's models for this OS.</summary>
    public IReadOnlyList<CatalogueEntry> Models { get; private set; } = [];

    /// <summary>The last models.list failed: the list is not known, which is not the same as empty.</summary>
    public bool Failed { get; private set; }

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

    public CatalogueLine Line(Job job)
    {
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
        "fluidaudio-parakeet-tdt-0.6b-v3" or "fluidaudio-parakeet-tdt-0.6b-v3-offline" => "Parakeet TDT v3",
        "nemotron-3-diarization-q8" => "Nemotron-3-Diarization",
        "silero-vad-v6" => "Silero VAD",
        _ => id,
    };

    /// <summary>A downloadable model's line: name, licence, size, and whether it is installed.</summary>
    public static string Downloadable(CatalogueEntry entry, IFormatProvider? format = null)
    {
        ArgumentNullException.ThrowIfNull(entry);
        return $"{Name(entry.Id)} · {entry.Licence} · {StorageModel.Size(entry.SizeBytes, format)} · {(entry.Installed ? "installed" : "not installed")}";
    }

    /// <summary>Whether this screen shows the failure (models.list); engine.route's are logged.</summary>
    public static bool Handles(CommandFailed failed)
    {
        ArgumentNullException.ThrowIfNull(failed);
        return failed.Command == "models.list";
    }

    public void Apply(InkEvent e)
    {
        switch (e)
        {
            case ModelsListed listed:
                Models = listed.Models;
                Failed = false;
                Changed();
                break;
            case CommandFailed failed when failed.Command == "models.list":
                Failed = true;
                Changed();
                break;
            case EngineRouted routed:
                serving = serving.SetItem(routed.Job, routed);
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
            case ModelUpdateFinished:
                Requery();
                break;
            case CoreStopped:
                shellEngines = shellEngines.Clear();
                serving = serving.Clear();
                Changed();
                break;
            default:
                break;
        }
    }
}
