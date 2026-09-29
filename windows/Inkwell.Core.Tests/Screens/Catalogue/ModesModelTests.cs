// Settings > Modes (as the Mac's ModesModelTests): every app named, never by its raw identity.
using System.Text.Json;
using System.Text.RegularExpressions;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

/// <summary>A directory with one app installed.</summary>
internal sealed class FakeApps : IAppDirectory
{
    public List<string> Asked { get; } = [];

    public InstalledApp? App(string exe)
    {
        Asked.Add(exe);
        return string.Equals(exe, "examplewriter.exe", StringComparison.OrdinalIgnoreCase)
            ? new InstalledApp("Example Writer", @"C:\Apps\ExampleWriter\examplewriter.exe")
            : null;
    }
}

public partial class ModesModelTests
{
    [GeneratedRegex(@"^[A-Za-z0-9_-]+\.exe$", RegexOptions.IgnoreCase)]
    private static partial Regex ExeName();

    [GeneratedRegex(@"^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$")]
    private static partial Regex BundleId();

    private static readonly string[] Identities =
    [
        "examplewriter.exe", "WhatsApp.exe", "unknowntool.exe", "slack", "com.example.missing.app",
        " zoom.exe ", "SLACK.EXE", "ms-teams.exe",
    ];

    private static string Listed() =>
        $$"""{"type":"modes.listed","default_id":"d","modes":[{"id":"chat","name":"Chat","style":"casual","polish":false,"remove_fillers":true,"apps":{{JsonSerializer.Serialize(Identities)}}},{"id":"d","name":"Default","style":"formal","polish":true,"remove_fillers":true,"apps":[]}]}""";

    /// <summary>
    /// Verify: no mode shows a raw exe name where a name is known, and an unknown one reads as its
    /// stem. (The Mac's NoModeShowsARawBundleID; its second pass over the real directory needs the
    /// Win32 directory, not written here: see NoInstalledApps.)
    /// </summary>
    [Fact]
    public void NoModeShowsARawExeName()
    {
        foreach (IAppDirectory directory in new IAppDirectory[] { new FakeApps(), NoInstalledApps.Instance })
        {
            var modes = new ModesModel(_ => { }, directory);
            modes.Apply(Ev.Of(Listed()));
            var shown = modes.Rows.SelectMany(r => new[] { r.Name, r.AppsText, r.AccessibilityLabel }.Concat(r.Traits).Concat(r.Apps.Select(a => a.Name))).ToList();
            Assert.NotEmpty(shown);
            foreach (var text in shown)
            {
                Assert.False(ExeName().IsMatch(text), $"{text} is an exe name");
                Assert.False(BundleId().IsMatch(text), $"{text} is a bundle id");
                Assert.DoesNotContain(".exe", text, StringComparison.OrdinalIgnoreCase);
                Assert.DoesNotContain("com.example", text, StringComparison.OrdinalIgnoreCase);
            }
        }
        var model = new ModesModel(_ => { }, new FakeApps());
        model.Apply(Ev.Of(Listed()));
        Assert.Equal(
            ["Example Writer", "WhatsApp", "Unknowntool", "Slack", "An app not on this PC", "Zoom", "Slack", "Microsoft Teams"],
            model.Rows[0].Apps.Select(a => a.Name));
        Assert.Equal(["Casual", "Clean up speech"], model.Rows[0].Traits);
        Assert.Equal(["Formal", "Clean up speech", "Polish"], model.Rows[1].Traits);
        Assert.True(model.Rows[1].IsDefault); // the default is listed last
        Assert.Equal("Everywhere else", model.Title(model.Rows[1]));
        Assert.Equal("Every app no other mode names", model.Rows[1].AppsText);
    }

    /// <summary>
    /// An installed app is named and iconed by the directory. (The Mac asked the real workspace for
    /// Finder; Windows asks the fake: the Win32 directory is the app's, not written here.)
    /// </summary>
    [Fact]
    public void AnInstalledAppIsNamedAndIconedByTheWorkspace()
    {
        var apps = new FakeApps();
        var writer = AppIdentity.Label(" ExampleWriter.exe ", apps);
        Assert.Equal("Example Writer", writer.Name);
        Assert.True(writer.Installed);
        Assert.NotNull(writer.IconPath);
        Assert.Equal("ExampleWriter.exe", writer.Id);
        var fragment = AppIdentity.Label("mail", apps);
        Assert.Equal("Mail", fragment.Name);
        Assert.DoesNotContain("mail", apps.Asked); // only an exe name is looked up
    }

    [Fact]
    public void AFailedListSaysSoAndAnAnswerClearsIt()
    {
        var sent = new Sent();
        var modes = new ModesModel(sent.Send);
        modes.Load();
        Assert.Equal([new CoreCommand.ModesList()], sent.Commands);
        var failed = Ev.Of<Inkwell.Core.Events.CommandFailed>("""{"type":"command.failed","command":"modes.list","message":"x"}""");
        modes.Apply(failed);
        Assert.True(modes.Failed);
        Assert.True(ModesModel.Handles(failed));
        modes.Apply(Ev.Of(Listed()));
        Assert.False(modes.Failed);
    }
}
