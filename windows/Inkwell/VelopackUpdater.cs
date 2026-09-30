// Inkwell's updates on Windows, through Velopack (MIT): the installer (Setup.exe, made by vpk in
// win-release.yml) installs Inkwell for the current user under %LOCALAPPDATA%\InkwellApp, and this
// reads the release feed (releases.win.json) of the newest published releases of SirSicard/inkwell
// on GitHub that carry one. The library is elsewhere (%LOCALAPPDATA%\Inkwell, DataLocation), so
// neither an update nor an uninstall touches it.
//
// What is checked, with no signing key (the Windows build is not code-signed yet): the feed and the
// packages come over HTTPS from GitHub, and Velopack refuses a downloaded package whose size or
// SHA-256 differs from what the feed says. Anyone who can publish a release on the repository can
// therefore ship an update, as they could ship an installer: docs/RELEASING.md.
//
// The row drives it (UpdatesModel): nothing is fetched until the user presses Check Now. A check
// asks GitHub's API for the repository's latest releases (unauthenticated: 60 requests an hour per
// address), then downloads the feed; it sends nothing about the PC but what any HTTPS request does.
using Inkwell.Core.Screens;
using Velopack;
using Velopack.Sources;

namespace Inkwell;

/// <param name="quit">The app's Quit: the core stops, then the process ends.</param>
internal sealed class VelopackUpdater(Action quit) : IUpdater
{
    /// <summary>Where the releases are.</summary>
    public const string Repository = "https://github.com/SirSicard/inkwell";

    private readonly UpdateManager manager = new(new GithubSource(Repository, accessToken: null, prerelease: false));
    private UpdateInfo? found;

    public bool UpdatesItself => manager.IsInstalled;

    public async Task<string?> CheckAsync()
    {
        found = await manager.CheckForUpdatesAsync().ConfigureAwait(false);
        return found?.TargetFullRelease.Version.ToString();
    }

    public Task DownloadAsync(Action<int> progress)
    {
        var update = found ?? throw new InvalidOperationException("no newer version was found to download");
        return manager.DownloadUpdatesAsync(update, progress);
    }

    public void RestartToUpdate()
    {
        var update = found ?? throw new InvalidOperationException("no update was downloaded");
        // Velopack's updater waits for this process to end, installs, and starts Inkwell again; the
        // app quits the usual way meanwhile, so the core stops and unloads its models first.
        manager.WaitExitThenApplyUpdates(update.TargetFullRelease, silent: false, restart: true);
        quit();
    }
}
