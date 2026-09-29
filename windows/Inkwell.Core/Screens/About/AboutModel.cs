// Settings > About: the version line, then every notice the Windows app ships, as the Mac's
// AboutSection (SettingsScreen.swift) lists them: the models, the code, and the Rust crates under
// one disclosure. Static data: nothing here changes while the app runs, so it sends no command and
// applies no event. The core's status line, which the Mac's About shows under the version, is the
// shell's (CoreStatus); the Mac's updates row has no Windows counterpart yet (no updater).

namespace Inkwell.Core.Screens;

/// <summary>
/// One of About's rows: a title, a line under it, and the text it opens onto (null: the row does
/// not open, as a model whose licence asks for no notice).
/// </summary>
public sealed record NoticeRow(string Title, string Detail, string? Text);

/// <summary>What Settings > About shows. UI thread only, like every screen model.</summary>
public sealed class AboutModel
{
    /// <summary>The disclosure's caption under <see cref="RustHeading"/>.</summary>
    public const string RustCaption = "The open-source crates compiled into Inkwell's core, each with its licence.";

    /// <param name="appVersion">
    /// The app's version (an assembly's informational version: build metadata after a "+" is
    /// dropped). Null or blank: a development build, as on the Mac without a bundle version.
    /// </param>
    public AboutModel(string? appVersion)
    {
        var version = appVersion?.Split('+')[0].Trim();
        VersionLine = $"Inkwell {(string.IsNullOrEmpty(version) ? "development build" : version)}";
    }

    /// <summary>"Inkwell 1.0.0", or "Inkwell development build".</summary>
    public string VersionLine { get; }

    /// <summary>The weights, each credited; a CC-BY model's row opens onto its credit.</summary>
    public IReadOnlyList<NoticeRow> ModelRows { get; } =
        [.. Notices.Models.Select(m => new NoticeRow(m.Title, m.Use, m.Notice))];

    /// <summary>The code that ships inside Inkwell, each row opening onto its licence.</summary>
    public IReadOnlyList<NoticeRow> ComponentRows { get; } =
        [.. Notices.Components.Select(c => new NoticeRow(c.Title, c.Role, c.Text))];

    /// <summary>The Rust crates' disclosure: "Rust libraries (N)".</summary>
    public string RustHeading { get; } = RustNotices.Heading;

    /// <summary>The Rust crates, in name order, each row opening onto its licence files.</summary>
    public IReadOnlyList<NoticeRow> RustRows { get; } =
        [.. RustNotices.Crates.Select(c => new NoticeRow(c.Title, c.Detail, c.Text))];
}
