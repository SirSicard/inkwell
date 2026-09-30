// Downloading the catalogue's models (Settings > Models and the first run's models step): nothing
// downloads until the user asks, one model.update at a time with model == next, progress and
// failures on the model's row, and the list asked again after each.
using System.Globalization;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class CatalogueDownloadTests
{
    private const string Qwen = "qwen3-asr-1.7b-q8";
    private const string Silero = "silero-vad-v6-16k";
    private const string Nemotron = "nemotron-3-diarization-q8";

    private static readonly CultureInfo Invariant = CultureInfo.InvariantCulture;

    /// <summary>Two models not installed (one from Hugging Face, one from GitHub) and one installed.</summary>
    internal static InkEvent Listed(bool qwenInstalled = false, bool sileroInstalled = false) => Ev.Of($$"""
        {"type":"models.listed","models":[
          {"id":"{{Qwen}}","licence":"Apache-2.0","size_bytes":2500000000,"installed":{{Bool(qwenInstalled)}},"jobs":[{"job":"dictation_final","wer":4.59}]},
          {"id":"{{Silero}}","licence":"MIT","size_bytes":1289603,"installed":{{Bool(sileroInstalled)}},"jobs":[{"job":"voice_activity","wer":1.5}]},
          {"id":"{{Nemotron}}","licence":"OpenMDW-1.1","size_bytes":107012128,"installed":true,"jobs":[{"job":"diarization","wer":20.2}]}]}
        """);

    internal static InkEvent Progress(string id, long done, long total) =>
        Ev.Of($$"""{"type":"model.update_progress","id":"{{id}}","next":"{{id}}","done_bytes":{{done}},"total_bytes":{{total}}}""");

    internal static InkEvent Finished(string id, bool ok, string? message = null) => Ev.Of(message is null
        ? $$"""{"type":"model.update_finished","id":"{{id}}","next":"{{id}}","ok":{{Bool(ok)}},"no_model_warm":false}"""
        : $$"""{"type":"model.update_finished","id":"{{id}}","next":"{{id}}","ok":{{Bool(ok)}},"no_model_warm":false,"message":"{{message}}"}""");

    private static string Bool(bool b) => b ? "true" : "false";

    private static ModelRow Row(CatalogueModel catalogue, string id) => catalogue.Rows.Single(r => r.Id == id);

    private static List<string> Updates(Sent sent) => sent.Commands.OfType<CoreCommand.ModelUpdate>().Select(u => u.Next).ToList();

    [Fact]
    public void NothingDownloadsUntilTheUserAsksAndThenOnlyWhatWasAskedFor()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Requery();
        catalogue.Apply(Listed());
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"dictation_final"}"""));
        catalogue.Apply(Ev.Of("""{"type":"engine.registered","id":"example-engine","kind":"streaming","jobs":[]}"""));
        Assert.Empty(Updates(sent)); // the list, the routes and an engine coming download nothing
        Assert.Equal([Qwen, Silero], catalogue.NotAskedFor.Select(m => m.Id)); // the installed one is not offered

        catalogue.Download(Qwen);
        var update = Assert.IsType<CoreCommand.ModelUpdate>(sent.Commands[^1]);
        Assert.Equal(new CoreCommand.ModelUpdate(Qwen, Qwen), update); // model == next: the first download
        Assert.Equal($$"""{"cmd":"model.update","id":"model.update:{{Qwen}}","model":"{{Qwen}}","next":"{{Qwen}}"}""", update.Json);
        Assert.Equal([Qwen], Updates(sent));
        Assert.Equal([Silero], catalogue.NotAskedFor.Select(m => m.Id));
        Assert.True(catalogue.Downloading);
    }

    [Fact]
    public void DownloadsRunOneAtATimeAndEachEndAsksTheListAndTheRoutesAgain()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(Listed());
        catalogue.DownloadMissing();
        Assert.Equal([Qwen], Updates(sent)); // Silero waits for Qwen
        Assert.Equal("Starting the download…", Row(catalogue, Qwen).Status(Invariant));
        Assert.Equal((double?)0, Row(catalogue, Qwen).Progress);
        Assert.IsType<ModelDownload.Waiting>(Row(catalogue, Silero).Download);
        Assert.Equal("Waiting for the download before it", Row(catalogue, Silero).Status(Invariant));
        Assert.Null(Row(catalogue, Silero).Progress);

        catalogue.Apply(Ev.Of($$"""{"type":"model.update_started","id":"{{Qwen}}","next":"{{Qwen}}"}"""));
        catalogue.Apply(Progress(Qwen, 1_288_490_189, 2_500_000_000));
        Assert.Equal("1.20 GB of 2.32 GB", Row(catalogue, Qwen).Status(Invariant));
        Assert.Equal(0.515, Row(catalogue, Qwen).Progress!.Value, 3);
        Assert.Equal([Qwen], Updates(sent));

        var before = sent.Commands.Count;
        catalogue.Apply(Finished(Qwen, ok: true));
        // The list and the routes first (engine.route waits behind a model.update), then the
        // dictation model kept warm, then the next download.
        CoreCommand[] afterQwen =
        [
            new CoreCommand.ModelsList(), new CoreCommand.EngineRoute(Job.DictationFinal), new CoreCommand.EngineRoute(Job.MeetingFinal),
            new CoreCommand.EngineRoute(Job.LivePartials), new CoreCommand.ModelWarm(Job.DictationFinal), new CoreCommand.ModelUpdate(Silero, Silero),
        ];
        Assert.Equal(afterQwen, sent.Commands.Skip(before));
        var qwen = Row(catalogue, Qwen);
        Assert.True(qwen.Installed); // before the list says so too
        Assert.False(qwen.CanDownload);
        Assert.Null(qwen.Status(Invariant));
        Assert.Equal("Qwen3-ASR 1.7B · Apache-2.0 · 2.32 GB · installed", qwen.Text(Invariant));

        catalogue.Apply(Listed(qwenInstalled: true));
        catalogue.Apply(Progress(Silero, 1_289_603, 1_289_603));
        catalogue.Apply(Finished(Silero, ok: true));
        Assert.Equal([Qwen, Silero], Updates(sent));
        Assert.False(catalogue.Downloading);
        Assert.Equal(2, catalogue.Requeries); // once after each
    }

    /// <summary>
    /// A model that does dictation, once this screen downloaded it, is kept warm (model.warm, as
    /// on the Mac): the first dictation after the download is not a cold load. The warm goes
    /// before the next download, so it never waits for that one. A model with no dictation job
    /// (Silero VAD) and a failed download are not warmed.
    /// </summary>
    [Fact]
    public void AnInstalledDictationModelIsKeptWarmBeforeTheNextDownload()
    {
        CoreCommand[] asked =
        [
            new CoreCommand.ModelsList(), new CoreCommand.EngineRoute(Job.DictationFinal),
            new CoreCommand.EngineRoute(Job.MeetingFinal), new CoreCommand.EngineRoute(Job.LivePartials),
        ];
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(Listed());
        catalogue.Download(Qwen);
        catalogue.Download(Silero);
        var before = sent.Commands.Count;
        catalogue.Apply(Finished(Qwen, ok: true));
        CoreCommand[] afterQwen = [.. asked, new CoreCommand.ModelWarm(Job.DictationFinal), new CoreCommand.ModelUpdate(Silero, Silero)];
        Assert.Equal(afterQwen, sent.Commands.Skip(before));
        before = sent.Commands.Count;
        catalogue.Apply(Finished(Silero, ok: true));
        Assert.Equal(asked, sent.Commands.Skip(before)); // Silero VAD does no dictation: not warmed

        var failing = new CatalogueModel(sent.Send);
        failing.Apply(Listed());
        failing.Download(Qwen);
        before = sent.Commands.Count;
        failing.Apply(Finished(Qwen, ok: false, "the connection was reset"));
        Assert.Equal(asked, sent.Commands.Skip(before)); // a failed download is not warmed
    }

    [Fact]
    public void AFailedDownloadSaysWhyInWordsAndRetryAsksAgainInItsTurn()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(Listed());
        catalogue.DownloadMissing();
        catalogue.Apply(Progress(Qwen, 1_000_000, 2_500_000_000));
        catalogue.Apply(Finished(Qwen, ok: false, "the new model could not be installed: downloading Qwen3-ASR-1.7B-Q8_0.gguf: connection reset"));
        var qwen = Row(catalogue, Qwen);
        Assert.True(qwen.CanRetry);
        Assert.False(qwen.CanDownload);
        Assert.Null(qwen.Progress);
        Assert.Equal(
            "Couldn't download Qwen3-ASR 1.7B: the new model could not be installed: downloading Qwen3-ASR-1.7B-Q8_0.gguf: connection reset",
            qwen.Status(Invariant));
        Assert.Equal("Retry downloading Qwen3-ASR 1.7B", qwen.RetryName);
        Assert.Equal([Qwen, Silero], Updates(sent)); // the next one still goes

        catalogue.Download(Qwen); // Retry, while Silero runs: it waits its turn
        Assert.IsType<ModelDownload.Waiting>(Row(catalogue, Qwen).Download);
        Assert.Equal([Qwen, Silero], Updates(sent));
        catalogue.Apply(Finished(Silero, ok: false)); // no reason given
        Assert.Equal("Couldn't download Silero VAD: the core gave no reason", Row(catalogue, Silero).Status(Invariant));
        Assert.Equal([Qwen, Silero, Qwen], Updates(sent));
    }

    [Fact]
    public void AnUpdateTheCoreRefusedOrNeverGotIsAFailureOnItsRow()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(Listed());
        catalogue.DownloadMissing();
        var other = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"model.update","id":"model.update:another-model","message":"x"}""");
        catalogue.Apply(other);
        Assert.IsType<ModelDownload.Running>(Row(catalogue, Qwen).Download); // another model's failure is not this one's

        var refused = Ev.Of<CommandFailed>($$"""{"type":"command.failed","command":"model.update","id":"model.update:{{Qwen}}","message":"model {{Qwen}} is being updated"}""");
        Assert.True(CatalogueModel.Handles(refused));
        catalogue.Apply(refused);
        Assert.Equal($"Couldn't download Qwen3-ASR 1.7B: model {Qwen} is being updated", Row(catalogue, Qwen).Status(Invariant));
        Assert.Equal([Qwen, Silero], Updates(sent)); // the next one goes

        // A command that never reached the core ends the same way, instead of waiting forever.
        var notSent = new CoreCommand.ModelUpdate(Silero, Silero).NotSent("couldn't send it: the core is not running");
        catalogue.Apply(notSent);
        Assert.True(Row(catalogue, Silero).CanRetry);
        Assert.False(catalogue.Downloading);
    }

    [Fact]
    public void AModelInstalledUnknownOrAlreadyAskedForIsNotAskedForAgain()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Download(Qwen); // before the list: nothing is known to download
        catalogue.Apply(Listed());
        catalogue.Download(Nemotron); // installed
        catalogue.Download("not-in-the-catalogue");
        Assert.Empty(Updates(sent));
        catalogue.Download(Qwen);
        catalogue.Download(Qwen); // running already
        catalogue.Download(Silero);
        catalogue.Download(Silero); // waiting already
        catalogue.DownloadMissing(); // nothing left that was not asked for
        Assert.Equal([Qwen], Updates(sent));
        catalogue.Apply(Finished(Qwen, ok: true));
        catalogue.Download(Qwen); // installed by this run's download
        Assert.Equal([Qwen, Silero], Updates(sent));
        catalogue.Apply(Progress(Qwen, 5, 10)); // a model that is not downloading moves nothing
        Assert.Null(Row(catalogue, Qwen).Progress);
    }

    [Fact]
    public void TheCoreStoppingEndsTheDownloadsWithWordsAndRetry()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(Listed());
        catalogue.DownloadMissing();
        catalogue.Apply(Ev.Of("""{"type":"core.stopped"}"""));
        Assert.False(catalogue.Downloading);
        Assert.Equal("Couldn't download Qwen3-ASR 1.7B: the core stopped", Row(catalogue, Qwen).Status(Invariant));
        Assert.True(Row(catalogue, Silero).CanRetry); // the one waiting too
        catalogue.Apply(Finished(Qwen, ok: true)); // a late answer from the stopped core changes no row
        Assert.True(Row(catalogue, Qwen).CanRetry);
        Assert.Equal([Qwen], Updates(sent));
        catalogue.Download(Silero); // Retry
        Assert.Equal([Qwen, Silero], Updates(sent));
    }

    /// <summary>
    /// Rows, Downloads and failures name a model, never show its id. The ids are the core's
    /// (registry.rs, rows.rs); the Mac's Parakeet row is macOS-only, so Windows never lists it.
    /// </summary>
    [Fact]
    public void EveryRegistryModelIsNamed()
    {
        foreach (var id in new[] { Qwen, Silero, Nemotron })
        {
            Assert.NotEqual(id, CatalogueModel.Name(id));
        }
    }

    /// <summary>What a download fetches is named before the user agrees: the model, its size, and where it comes from.</summary>
    [Fact]
    public void EveryDownloadSaysWhereItComesFrom()
    {
        Assert.Equal("huggingface.co", CatalogueModel.Source(Qwen));
        Assert.Equal("huggingface.co", CatalogueModel.Source(Nemotron));
        Assert.Equal("GitHub", CatalogueModel.Source(Silero)); // ink-engines' rows.rs: raw.githubusercontent.com
        var catalogue = new CatalogueModel(_ => { });
        catalogue.Apply(Listed());
        var qwen = Row(catalogue, Qwen);
        Assert.Equal("From huggingface.co", qwen.From);
        Assert.Equal("Download Qwen3-ASR 1.7B, 2.32 GB, from huggingface.co", qwen.DownloadName(Invariant));
        Assert.Equal("From GitHub", Row(catalogue, Silero).From);
        Assert.Null(Row(catalogue, Nemotron).From); // installed: nothing to download
        catalogue.Download(Qwen);
        Assert.Null(Row(catalogue, Qwen).From); // asked for: its progress shows instead
        Assert.Equal("Downloading Qwen3-ASR 1.7B", Row(catalogue, Qwen).ProgressName);
    }
}
