// The first run's terms step (Windows only): shown until agreed to, remembered with the terms'
// version, asked again when the terms change, and nothing started before Agree or on Quit.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public sealed class TermsStepTests : IDisposable
{
    private readonly string root = Path.Combine(Path.GetTempPath(), $"inkwell-terms-{Guid.NewGuid():N}");
    private int started;
    private int quit;

    public void Dispose()
    {
        if (Directory.Exists(root))
        {
            Directory.Delete(root, recursive: true);
        }
    }

    private string RecordPath => Path.Combine(root, TermsRecord.FileName);

    private TermsStep Step(TermsRecord? record, Logged? logged = null, string? version = null) =>
        new(record, () => started++, () => quit++, logged?.Log ?? new Logged().Log, version);

    [Fact]
    public void UntilTheTermsAreAgreedToTheStepShowsAndNothingStarts()
    {
        var step = Step(new TermsRecord(RecordPath));
        Assert.False(step.Showing); // nothing decided before Launch
        step.Launch();
        Assert.True(step.Showing);
        Assert.Equal(0, started);
        Assert.Equal(0, quit);
        Assert.False(File.Exists(RecordPath)); // nothing recorded by showing it
    }

    [Fact]
    public void AgreeIsRecordedWithTheTermsVersionAndTheNextLaunchStartsWithoutTheStep()
    {
        var step = Step(new TermsRecord(RecordPath));
        step.Launch();
        step.Agree();
        Assert.False(step.Showing);
        Assert.Equal(1, started);
        Assert.Equal(TermsStep.CurrentVersion, File.ReadAllText(RecordPath).Trim());
        step.Agree(); // a second press starts nothing more
        step.Quit(); // nor does Quit once agreed
        Assert.Equal(1, started);
        Assert.Equal(0, quit);

        var next = Step(new TermsRecord(RecordPath));
        next.Launch();
        Assert.False(next.Showing);
        Assert.Equal(2, started); // started at once, no step
    }

    [Fact]
    public void ChangedTermsAskAgain()
    {
        var first = Step(new TermsRecord(RecordPath), version: "sha256:old");
        first.Launch();
        first.Agree();
        Assert.Equal(1, started);

        var changed = Step(new TermsRecord(RecordPath), version: "sha256:new");
        changed.Launch();
        Assert.True(changed.Showing); // agreed to other terms: asked again
        Assert.Equal(1, started);
        changed.Agree();
        Assert.Equal("sha256:new", File.ReadAllText(RecordPath).Trim());

        // The version is the terms' own: a change to the sentence or either licence changes it.
        var current = TermsStep.VersionOf(Notices.WindowsAppSdkTerms, Notices.WindowsAppSdkLicence, Notices.WindowsSdkNetText);
        Assert.Equal(current, TermsStep.CurrentVersion);
        Assert.NotEqual(current, TermsStep.VersionOf(Notices.WindowsAppSdkTerms + " ", Notices.WindowsAppSdkLicence, Notices.WindowsSdkNetText));
        Assert.NotEqual(current, TermsStep.VersionOf(Notices.WindowsAppSdkTerms, Notices.WindowsAppSdkLicence, Notices.WindowsSdkNetText + " "));
    }

    [Fact]
    public void QuitExitsWithNothingStartedOrRecorded()
    {
        var step = Step(new TermsRecord(RecordPath));
        step.Launch();
        step.Quit();
        Assert.False(step.Showing);
        Assert.Equal(1, quit);
        Assert.Equal(0, started);
        step.Agree(); // too late: the app is exiting
        Assert.Equal(0, started);
        Assert.False(File.Exists(RecordPath));

        var next = Step(new TermsRecord(RecordPath));
        next.Launch();
        Assert.True(next.Showing); // asked again at the next launch
    }

    [Fact]
    public void ARecordThatCannotBeReadShowsTheStepAndOneThatCannotBeWrittenStillStarts()
    {
        // Unreadable: the record's path is a folder.
        Directory.CreateDirectory(RecordPath);
        var logged = new Logged();
        var unreadable = Step(new TermsRecord(RecordPath), logged);
        unreadable.Launch();
        Assert.True(unreadable.Showing);
        Assert.Contains(logged.Messages, m => m.StartsWith("couldn't read the terms agreement", StringComparison.Ordinal));

        // Unwritable: the user agreed, so the app starts; the failure is logged.
        unreadable.Agree();
        Assert.Equal(1, started);
        Assert.Contains(logged.Messages, m => m.StartsWith("couldn't record the terms agreement", StringComparison.Ordinal));

        // No library folder known: the step shows at every launch, and Agree still starts the app.
        var unknown = new Logged();
        var nowhere = Step(null, unknown);
        nowhere.Launch();
        Assert.True(nowhere.Showing);
        nowhere.Agree();
        Assert.Equal(2, started);
        Assert.Single(unknown.Messages);
    }

    [Fact]
    public void TheStepShowsTheTermsSentenceAndBothMicrosoftLicencesAsAboutDoes()
    {
        Assert.Equal(AboutModel.Terms, TermsStep.Sentence);
        var about = new AboutModel(null).ComponentRows;
        Assert.Equal(2, TermsStep.Licences.Count);
        Assert.All(TermsStep.Licences, row => Assert.Contains(row, about)); // the same title, line and full text
        Assert.Contains("MICROSOFT SOFTWARE LICENSE TERMS\nMICROSOFT WINDOWS APP SDK", TermsStep.Licences[0].Text, StringComparison.Ordinal);
        Assert.Contains("MICROSOFT WINDOWS SOFTWARE DEVELOPMENT KIT (SDK)", TermsStep.Licences[1].Text, StringComparison.Ordinal);
        // The Windows SDK's Distributable Code that Inkwell ships is two files, both on its REDIST
        // list: the row the user agrees under names each (C#/WinRT's own row says MIT first).
        foreach (var file in new[] { "Microsoft.Windows.SDK.NET.dll", "WinRT.Runtime.dll" })
        {
            Assert.Contains(file, TermsStep.Licences[1].Detail, StringComparison.Ordinal);
        }
        // The sentence names both, by their rows' names.
        foreach (var id in new[] { "windows-app-sdk", "windows-sdk-net" })
        {
            Assert.Contains($"\"{Notices.Components.Single(c => c.Id == id).Name}\"", TermsStep.Sentence, StringComparison.Ordinal);
        }
        Assert.Equal("Agree", TermsStep.AgreeTitle);
        Assert.Equal("Quit", TermsStep.QuitTitle);
    }
}
