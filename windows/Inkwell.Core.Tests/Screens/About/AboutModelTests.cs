// Settings > About's rows, as the Mac's AboutSection composes them (no Mac test covers the view;
// these hold the Windows model to its texts).
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class AboutModelTests
{
    [Fact]
    public void TheVersionLineNamesTheVersionOrADevelopmentBuild()
    {
        Assert.Equal("Inkwell 1.2.0", new AboutModel("1.2.0").VersionLine);
        Assert.Equal("Inkwell 1.2.0", new AboutModel("1.2.0+4f2c9e1").VersionLine); // build metadata dropped
        Assert.Equal("Inkwell development build", new AboutModel(null).VersionLine);
        Assert.Equal("Inkwell development build", new AboutModel(" ").VersionLine);
    }

    [Fact]
    public void TheCopyrightLineIsTheLicencesAndInkwellExeCarriesIt()
    {
        // The Windows SDK's licence (Distributable Code, Distribution Requirements): an app that ships
        // its code displays its own valid copyright notice. It is LICENSE's first copyright line,
        // Inkwell's (the second covers the legacy 0.2 app, which the Windows build does not
        // contain), shown under the version; Inkwell.exe's file properties carry it too
        // (Inkwell.csproj's Copyright).
        var root = AboutCheckout.Root();
        var licence = File.ReadAllLines(Path.Combine(root, "LICENSE"));
        Assert.Equal(AboutModel.CopyrightLine, licence.First(l => l.StartsWith("Copyright", StringComparison.Ordinal)));
        Assert.Contains(
            $"<Copyright>{AboutModel.CopyrightLine}</Copyright>",
            File.ReadAllText(Path.Combine(root, "windows", "Inkwell", "Inkwell.csproj")),
            StringComparison.Ordinal);
    }

    [Fact]
    public void TheTermsForTheWindowsAppSdkAreAgreedToAndPointAtItsLicence()
    {
        // Section 3(b)(ii) of the Windows App SDK's licence: end users agree to terms that protect
        // its code at least as much; these are its own terms, whose full text is a notice below.
        Assert.Contains("Microsoft Software License Terms", AboutModel.Terms, StringComparison.Ordinal);
        Assert.Contains("you agree to those terms", AboutModel.Terms, StringComparison.Ordinal);
        Assert.Contains($"\"{Notices.Components.Single(c => c.Id == "windows-app-sdk").Name}\"", AboutModel.Terms, StringComparison.Ordinal);
        // And the Visual C++ runtime beside the app (the Visual Studio licence's Distributable Code).
        Assert.Contains($"\"{Notices.Components.Single(c => c.Id == "vc-runtime").Name}\"", AboutModel.Terms, StringComparison.Ordinal);
    }

    [Fact]
    public void TheRowsAreTheNoticesInOrderWithTheMacsTitles()
    {
        var about = new AboutModel(null);
        Assert.Equal(Notices.Models.Count, about.ModelRows.Count);
        Assert.Equal(Notices.Components.Count, about.ComponentRows.Count);
        Assert.Equal(RustNotices.Crates.Count, about.RustRows.Count);

        var parakeet = about.ModelRows[Notices.Models.ToList().FindIndex(m => m.Id == "parakeet")];
        Assert.Equal("Parakeet TDT 0.6B v3, by NVIDIA (CC-BY-4.0)", parakeet.Title);
        Assert.Contains("Creative Commons Attribution 4.0", parakeet.Text, StringComparison.Ordinal);
        Assert.Null(about.ModelRows[0].Text); // Qwen3-ASR's licence asks for no notice: the row does not open

        Assert.Equal("wasapi-rs, by Henrik Enquist (MIT)", about.ComponentRows[0].Title);
        Assert.Equal(Notices.Components[0].Role, about.ComponentRows[0].Detail);
        Assert.All(about.ComponentRows, r => Assert.NotNull(r.Text));

        var first = RustNotices.Crates[0];
        Assert.Equal(new NoticeRow(first.Title, first.Detail, first.Text), about.RustRows[0]);
        Assert.Equal($"Rust libraries ({RustNotices.Crates.Count})", about.RustHeading);
        Assert.Equal("The open-source crates compiled into Inkwell's core, each with its licence.", AboutModel.RustCaption);
    }
}
