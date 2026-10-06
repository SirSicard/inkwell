// Settings > Models (as the Mac's CatalogueModelTests): what the screen asks and shows.
using System.Globalization;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class CatalogueModelTests
{
    /// <summary>engine.routed answers only engine.route: the screen asks again when the answer may change.</summary>
    [Fact]
    public void TheScreenAsksAgainAfterAnInstallOrWhenAnEngineComesOrGoes()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        CoreCommand[] asked =
        [
            new CoreCommand.ModelsList(), new CoreCommand.EngineRoute(Job.DictationFinal),
            new CoreCommand.EngineRoute(Job.MeetingFinal), new CoreCommand.EngineRoute(Job.LivePartials),
        ];
        catalogue.Requery();
        Assert.Equal(asked, sent.Commands);
        catalogue.Apply(Ev.Of("""{"type":"model.update_finished","id":"qwen3-asr-1.7b-q8","next":"qwen3-asr-1.7b-q8","ok":true,"no_model_warm":false}"""));
        Assert.Equal([.. asked, .. asked], sent.Commands); // a model installed: ask again
        // Windows: a shell engine (the Mac's example was Apple Intelligence).
        catalogue.Apply(Ev.Of("""{"type":"engine.registered","id":"example-llm","kind":"llm","jobs":[]}"""));
        Assert.Equal(12, sent.Commands.Count); // an engine came: ask again
        catalogue.Apply(Ev.Of("""{"type":"engine.unregistered","id":"example-llm"}"""));
        Assert.Equal(16, sent.Commands.Count); // and when it went
        catalogue.Apply(Ev.Of("""{"type":"model.warmed","id":"qwen3-asr-1.7b-q8","job":"dictation_final"}"""));
        Assert.Equal(16, sent.Commands.Count); // warming changes no answer
        Assert.Equal(4, catalogue.Requeries);
    }

    /// <summary>
    /// The on-device language model is a row like the speech models (Settings > Models lists it,
    /// with its name), but Download all fetches speech models only: the language model is the
    /// user's own choice.
    /// </summary>
    [Fact]
    public void TheLanguageModelIsARowButNotPartOfDownloadAll()
    {
        var catalogue = new CatalogueModel(_ => { });
        catalogue.Apply(Ev.Of("""{"type":"models.listed","models":[{"id":"silero-vad-v6-16k","kind":"speech","licence":"MIT","size_bytes":1289603,"installed":false,"jobs":[{"job":"voice_activity","wer":1.5}]},{"id":"qwen3-4b-instruct-2507-q4km","kind":"language","name":"Qwen3 4B Instruct","licence":"Apache-2.0","size_bytes":2497281120,"installed":false,"jobs":[]},{"id":"qwen3-asr-1.7b-q8","licence":"Apache-2.0","size_bytes":2500000000,"installed":true,"jobs":[{"job":"dictation_final","wer":4.59}]}]}"""));
        Assert.Equal(["silero-vad-v6-16k", "qwen3-4b-instruct-2507-q4km", "qwen3-asr-1.7b-q8"], catalogue.Models.Select(m => m.Id));
        Assert.Equal("Qwen3 4B Instruct", catalogue.LanguageRow?.Name);
        catalogue.DownloadMissing();
        Assert.Equal(["silero-vad-v6-16k"], catalogue.Rows.Where(r => r.Download is not null).Select(r => r.Id)); // what Download all fetches
    }

    [Fact]
    public void AFailedListReadsAsFailedNotAsNothingInstalled()
    {
        var catalogue = new CatalogueModel(_ => { });
        var failed = Ev.Of<CommandFailed>("""{"type":"command.failed","command":"models.list","message":"the registry could not be read"}""");
        catalogue.Apply(failed);
        Assert.True(catalogue.Failed);
        Assert.True(CatalogueModel.Handles(failed));
        Assert.Equal("The model list could not be read.", CatalogueModel.FailedText);
        catalogue.Apply(Ev.Of("""{"type":"models.listed","models":[]}"""));
        Assert.False(catalogue.Failed); // a list that arrives clears it
    }

    [Fact]
    public void EachJobShowsWhatServesItAndItsMeasuredAccuracy()
    {
        var catalogue = new CatalogueModel(_ => { });
        Assert.False(catalogue.Line(Job.DictationFinal).Known);
        Assert.Equal("Checking…", catalogue.Line(Job.DictationFinal).EngineText);
        catalogue.Apply(Ev.Of("""{"type":"models.listed","models":[{"id":"qwen3-asr-1.7b-q8","licence":"Apache-2.0","size_bytes":2500000000,"installed":true,"jobs":[{"job":"dictation_final","wer":4.59},{"job":"meeting_final","wer":16.08}]}]}"""));
        catalogue.Apply(Ev.Of("""{"type":"engine.registered","id":"fluidaudio-parakeet-tdt-0.6b-v3","kind":"streaming","jobs":[{"job":"live_partials","wer":21.3}]}"""));
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"dictation_final","id":"qwen3-asr-1.7b-q8","source":"registry"}"""));
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"live_partials","id":"fluidaudio-parakeet-tdt-0.6b-v3","source":"shell"}"""));
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"meeting_final"}"""));
        var dictation = catalogue.Line(Job.DictationFinal);
        Assert.Equal("Qwen3-ASR 1.7B", dictation.Engine);
        Assert.Equal(4.59, dictation.Wer);
        Assert.Equal("95.4 % of words right (4.6 % word error rate)", dictation.Accuracy);
        Assert.Equal("Parakeet TDT v3", catalogue.Line(Job.LivePartials).Engine);
        Assert.Equal(21.3, catalogue.Line(Job.LivePartials).Wer);
        Assert.True(catalogue.Line(Job.MeetingFinal).Known);
        Assert.Null(catalogue.Line(Job.MeetingFinal).Engine); // nothing fills it
        Assert.Equal("Nothing installed yet", catalogue.Line(Job.MeetingFinal).EngineText);
        // Windows: the downloadable line sizes as Windows does.
        Assert.Equal("Qwen3-ASR 1.7B · Apache-2.0 · 2.32 GB · installed", CatalogueModel.Downloadable(catalogue.Models[0], CultureInfo.InvariantCulture));
        // Every registry id Windows lists has its name (ink-engines' rows.rs), never the raw id.
        Assert.Equal("Parakeet TDT v3", CatalogueModel.Name("parakeet-tdt-0.6b-v3-int8"));
        Assert.Equal("Nemotron-3-Diarization", CatalogueModel.Name("nemotron-3-diarization-q8"));
        Assert.Equal("Silero VAD", CatalogueModel.Name("silero-vad-v6-16k"));
        catalogue.Apply(Ev.Of("""{"type":"core.stopped"}"""));
        Assert.False(catalogue.Line(Job.DictationFinal).Known); // a stopped core's answers are gone
    }

    /// <summary>A route question that failed reads "couldn't", never "nothing installed", until its answer comes.</summary>
    [Fact]
    public void AFailedRouteQuestionSaysSoOnItsLine()
    {
        var catalogue = new CatalogueModel(new Sent().Send);
        var failed = new CoreCommand.EngineRoute(Job.MeetingFinal).NotSent("couldn't send it: the core is not running");
        Assert.True(CatalogueModel.Handles(failed));
        catalogue.Apply(failed);
        Assert.Equal(CatalogueLine.FailedText, catalogue.Line(Job.MeetingFinal).EngineText);
        Assert.Equal("Checking…", catalogue.Line(Job.DictationFinal).EngineText);
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"meeting_final"}"""));
        Assert.Equal("Nothing installed yet", catalogue.Line(Job.MeetingFinal).EngineText);
    }

    /// <summary>
    /// Whether a speech model is installed: any of dictation, meeting transcript and live words
    /// routed to a model. Not known until all three have answered; none routed is "no model".
    /// </summary>
    [Fact]
    public void ASpeechModelIsInstalledWhenAnySpeechJobHasOne()
    {
        var catalogue = new CatalogueModel(_ => { });
        Assert.Null(catalogue.HasSpeechModel);
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"dictation_final"}"""));
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"meeting_final"}"""));
        Assert.Null(catalogue.HasSpeechModel); // live words not answered yet
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"live_partials"}"""));
        Assert.False(catalogue.HasSpeechModel);
        catalogue.Apply(Ev.Of("""{"type":"engine.routed","job":"live_partials","id":"parakeet-tdt-0.6b-v3-int8","source":"registry"}"""));
        Assert.True(catalogue.HasSpeechModel);
    }

}
