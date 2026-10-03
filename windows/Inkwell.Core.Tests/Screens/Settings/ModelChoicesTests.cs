// The first run's models step as choices by outcome: what each takes (model, licence, size, host),
// the total on the Download button, nothing downloaded until it is pressed, and then only what is
// chosen, smallest first.
using System.Globalization;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class ModelChoicesTests
{
    private static readonly CultureInfo Invariant = CultureInfo.InvariantCulture;

    private static List<string> Updates(Sent sent) => sent.Commands.OfType<CoreCommand.ModelUpdate>().Select(u => u.Next).ToList();

    [Fact]
    public void EachChoiceSaysWhatItTakesAndTheButtonCarriesTheTotal()
    {
        var catalogue = new CatalogueModel(_ => { });
        catalogue.Apply(CatalogueDownloadTests.ListedWindows());
        var choices = new ModelChoices();
        Assert.Equal(["speech", "accuracy", "speakers"], ModelChoices.Shown(catalogue).Select(c => c.Id));
        Assert.Equal("Dictation, live words and meeting transcripts", ModelChoices.Speech.Title);
        Assert.Equal("Fewer mistakes", ModelChoices.Accuracy.Title);
        Assert.Equal("Tell the people on the call apart", ModelChoices.Speakers.Title);
        Assert.Equal(
            "Silero VAD (MIT) and Parakeet TDT v3 (CC-BY-4.0) · 640 MB from raw.githubusercontent.com and huggingface.co",
            ModelChoices.Line(ModelChoices.Speech, catalogue, Invariant));
        Assert.Equal("Qwen3-ASR 1.7B (Apache-2.0) · 2.32 GB from huggingface.co", ModelChoices.Line(ModelChoices.Accuracy, catalogue, Invariant));
        Assert.Equal("Nemotron-3-Diarization (OpenMDW-1.1) · 102 MB from huggingface.co", ModelChoices.Line(ModelChoices.Speakers, catalogue, Invariant));
        // Windows' honest lines: the live transcript is what a meeting keeps until the meeting model is in.
        Assert.Equal(
            "On Windows, a meeting keeps this live transcript until the meeting model, under Fewer mistakes, is added.",
            ModelChoices.Note(ModelChoices.Speech, catalogue));
        Assert.Equal("On Windows this is also the meeting model: it gives a meeting its final pass.", ModelChoices.Note(ModelChoices.Accuracy, catalogue));
        Assert.Null(ModelChoices.Note(ModelChoices.Speakers, catalogue));

        // The recommended set is always taken; the others start off.
        Assert.True(choices.IsTicked(ModelChoices.Speech, catalogue));
        Assert.False(ModelChoices.CanTick(ModelChoices.Speech, catalogue));
        Assert.False(choices.IsTicked(ModelChoices.Accuracy, catalogue));
        Assert.True(ModelChoices.CanTick(ModelChoices.Accuracy, catalogue));
        Assert.Equal("Download 640 MB", choices.DownloadTitle(catalogue, Invariant));
        Assert.Equal(
            "Download Silero VAD and Parakeet TDT v3: 640 MB in all, from raw.githubusercontent.com and huggingface.co",
            choices.DownloadName(catalogue, Invariant));
        Assert.All(ModelChoices.Shown(catalogue), c => Assert.Null(ModelChoices.Status(c, catalogue, Invariant)));
    }

    [Fact]
    public void TickingChangesTheTotalAndDownloadFetchesWhatIsChosenSmallestFirst()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(CatalogueDownloadTests.ListedWindows());
        var choices = new ModelChoices();
        var changes = 0;
        choices.PropertyChanged += (_, _) => changes++;
        choices.Tick(ModelChoices.Accuracy, on: true);
        Assert.Equal("Download 2.95 GB", choices.DownloadTitle(catalogue, Invariant));
        choices.Tick(ModelChoices.Speakers, on: true);
        Assert.Equal("Download 3.05 GB", choices.DownloadTitle(catalogue, Invariant));
        choices.Tick(ModelChoices.Speakers, on: false);
        choices.Tick(ModelChoices.Speech, on: false); // required: stays
        Assert.True(choices.IsTicked(ModelChoices.Speech, catalogue));
        Assert.Equal(3, changes);
        Assert.Empty(sent.Commands); // ticking downloads nothing

        choices.Download(catalogue);
        Assert.Equal(["silero-vad-v6-16k"], Updates(sent));
        Assert.Null(choices.DownloadTitle(catalogue, Invariant)); // all asked for: the button goes
        Assert.False(ModelChoices.CanTick(ModelChoices.Accuracy, catalogue)); // asked for: its tick stays
        Assert.True(choices.IsTicked(ModelChoices.Accuracy, catalogue));
        Assert.Equal("Waiting for the download before it", ModelChoices.Status(ModelChoices.Accuracy, catalogue, Invariant));
        catalogue.Apply(CatalogueDownloadTests.Progress("silero-vad-v6-16k", 1_000_000, 1_289_603));
        Assert.Equal("Silero VAD: 976 KB of 1.22 MB", ModelChoices.Status(ModelChoices.Speech, catalogue, Invariant));
        catalogue.Apply(CatalogueDownloadTests.Finished("silero-vad-v6-16k", ok: true));
        catalogue.Apply(CatalogueDownloadTests.Finished("parakeet-tdt-0.6b-v3-int8", ok: true));
        Assert.Equal(["silero-vad-v6-16k", "parakeet-tdt-0.6b-v3-int8", "qwen3-asr-1.7b-q8"], Updates(sent));
        Assert.Equal("Installed", ModelChoices.Status(ModelChoices.Speech, catalogue, Invariant));
        Assert.DoesNotContain(ModelChoices.DiarizerId, Updates(sent)); // not chosen
    }

    [Fact]
    public void AFailedDownloadSaysWhyAndDownloadRetriesIt()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(CatalogueDownloadTests.ListedWindows(parakeet: true));
        var choices = new ModelChoices();
        choices.Download(catalogue);
        catalogue.Apply(CatalogueDownloadTests.Finished("silero-vad-v6-16k", ok: false, message: "the network went away"));
        Assert.Equal("Couldn't download Silero VAD: the network went away", ModelChoices.Status(ModelChoices.Speech, catalogue, Invariant));
        Assert.Equal("Download 1.22 MB", choices.DownloadTitle(catalogue, Invariant));
        choices.Download(catalogue);
        Assert.Equal(["silero-vad-v6-16k", "silero-vad-v6-16k"], Updates(sent));
    }

    [Fact]
    public void WhatIsInstalledShowsTickedAndIsNotDownloadedAgain()
    {
        var catalogue = new CatalogueModel(_ => { });
        catalogue.Apply(CatalogueDownloadTests.ListedWindows(qwen: true, parakeet: true, silero: true));
        var choices = new ModelChoices();
        Assert.Equal("Installed", ModelChoices.Status(ModelChoices.Speech, catalogue, Invariant));
        Assert.Null(ModelChoices.Note(ModelChoices.Speech, catalogue)); // the meeting model is in
        Assert.True(choices.IsTicked(ModelChoices.Accuracy, catalogue));
        Assert.False(ModelChoices.CanTick(ModelChoices.Accuracy, catalogue));
        Assert.Null(choices.DownloadTitle(catalogue, Invariant)); // the diarizer is not ticked
        Assert.StartsWith("Inkwell turns speech into text with models that run on this PC. Choose what it should do", OnboardingModel.ModelsNote(catalogue), StringComparison.Ordinal);
        catalogue.Apply(CatalogueDownloadTests.ListedWindows(qwen: true, nemotron: true, parakeet: true, silero: true));
        Assert.Equal("Every model Inkwell uses is on this PC.", OnboardingModel.ModelsNote(catalogue));
    }

    /// <summary>A choice whose models the catalogue does not list (a core without that engine) is not offered.</summary>
    [Fact]
    public void OnlyChoicesTheCatalogueListsAreOffered()
    {
        var catalogue = new CatalogueModel(_ => { });
        catalogue.Apply(Ev.Of("""{"type":"models.listed","models":[{"id":"qwen3-asr-1.7b-q8","licence":"Apache-2.0","size_bytes":2500000000,"installed":false,"jobs":[]}]}"""));
        Assert.Equal(["accuracy"], ModelChoices.Shown(catalogue).Select(c => c.Id));
        Assert.Null(new ModelChoices().DownloadTitle(catalogue, Invariant)); // the required choice is not there; accuracy not ticked
    }

    /// <summary>Today's one action is still the recommended set alone.</summary>
    [Fact]
    public void TodaysDownloadIsTheRecommendedSet()
    {
        var sent = new Sent();
        var catalogue = new CatalogueModel(sent.Send);
        catalogue.Apply(CatalogueDownloadTests.ListedWindows());
        catalogue.DownloadRecommended();
        catalogue.Apply(CatalogueDownloadTests.Finished("silero-vad-v6-16k", ok: true));
        catalogue.Apply(CatalogueDownloadTests.Finished("parakeet-tdt-0.6b-v3-int8", ok: true));
        Assert.Equal(["silero-vad-v6-16k", "parakeet-tdt-0.6b-v3-int8"], Updates(sent));
    }
}
