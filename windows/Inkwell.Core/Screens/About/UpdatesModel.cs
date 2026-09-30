// Settings > About's updates row: the Windows counterpart of the Mac's (Sparkle's "Check Now"). The
// app's updater is Velopack's, reading the releases on GitHub (the app's VelopackUpdater); this
// model is what the row says and does, over any IUpdater, so it tests without one.
//
// Nothing happens unless the user asks: no check runs at start or on a timer (rule 9), and each
// step (check, download, restart) is the user's press. A failure says "Couldn't ..." with its
// reason, and the row offers the check again. The reasons are the updater's (a network or HTTP
// error, a checksum that does not match): they name URLs and files, never anything the user said.

namespace Inkwell.Core.Screens;

/// <summary>The app's updater, as the updates row drives it.</summary>
public interface IUpdater
{
    /// <summary>
    /// Whether this copy updates itself: true once installed by Inkwell's installer; false for a
    /// build run from a folder (a development build, or a publish not installed).
    /// </summary>
    bool UpdatesItself { get; }

    /// <summary>
    /// The newer version the release feed offers ("1.0.1"), or null when this is the newest. Throws
    /// when the feed cannot be read.
    /// </summary>
    Task<string?> CheckAsync();

    /// <summary>
    /// Downloads the version the last check found and checks it against the feed's checksum,
    /// reporting 0 to 100 on any thread. Throws when either fails.
    /// </summary>
    Task DownloadAsync(Action<int> progress);

    /// <summary>
    /// UI thread. Installs the downloaded version once the app has quit, starts it again, and
    /// quits the app the way its Quit does (the core stops first). Throws when the installer
    /// cannot be started; the app then keeps running.
    /// </summary>
    void RestartToUpdate();
}

/// <summary>The updater of a copy that does not update itself (tests, and where none is given).</summary>
public sealed class NoUpdater : IUpdater
{
    public static NoUpdater Instance { get; } = new();

    public bool UpdatesItself => false;

    public Task<string?> CheckAsync() => Task.FromResult<string?>(null);

    public Task DownloadAsync(Action<int> progress) => Task.CompletedTask;

    public void RestartToUpdate()
    {
    }
}

/// <summary>Where the updates row is.</summary>
public enum UpdateState
{
    /// <summary>This copy does not update itself (<see cref="IUpdater.UpdatesItself"/>).</summary>
    Off,
    /// <summary>Nothing checked yet.</summary>
    Idle,
    Checking,
    UpToDate,
    /// <summary>A newer version is offered, not downloaded.</summary>
    Available,
    Downloading,
    /// <summary>Downloaded and checked: a restart installs it.</summary>
    Ready,
    /// <summary>The check, the download or the restart failed (<see cref="UpdatesModel.Line"/> says why).</summary>
    Failed,
}

/// <summary>What the updates row shows and does. UI thread only, like every screen model.</summary>
public sealed class UpdatesModel(IUpdater updater, ScreenLog? log = null) : ObservableModel
{
    public const string OffText = "Updates work only in the installed app.";
    public const string CheckTitle = "Check Now";
    public const string DownloadTitle = "Download and Install";
    public const string RestartTitle = "Restart to Update";

    private readonly ScreenLog log = log ?? ScreenLog.System;
    private int percent;
    private string? failure;

    public UpdateState State { get; private set; } = updater.UpdatesItself ? UpdateState.Idle : UpdateState.Off;

    /// <summary>The version on offer, once a check found one.</summary>
    public string? Version { get; private set; }

    /// <summary>The row's line: where things are, or null when there is nothing to say.</summary>
    public string? Line => State switch
    {
        UpdateState.Off => OffText,
        UpdateState.Idle => null,
        UpdateState.Checking => "Checking for updates…",
        UpdateState.UpToDate => "Inkwell is up to date.",
        UpdateState.Available => $"Inkwell {Version} is available.",
        UpdateState.Downloading => $"Downloading Inkwell {Version}: {percent}%",
        UpdateState.Ready => $"Inkwell {Version} is downloaded. Inkwell restarts to install it.",
        UpdateState.Failed => failure,
        _ => null,
    };

    /// <summary>The row's button, or null where it has none (updates off).</summary>
    public string? ActionTitle => State switch
    {
        UpdateState.Off => null,
        UpdateState.Available or UpdateState.Downloading => DownloadTitle,
        UpdateState.Ready => RestartTitle,
        _ => CheckTitle,
    };

    /// <summary>Whether the button does anything now (not while a check or a download runs).</summary>
    public bool CanAct => State is not (UpdateState.Off or UpdateState.Checking or UpdateState.Downloading);

    /// <summary>The button: checks, downloads or restarts, as <see cref="ActionTitle"/> says.</summary>
    public Task Act() => State switch
    {
        UpdateState.Idle or UpdateState.UpToDate or UpdateState.Failed => Check(),
        UpdateState.Available => Download(),
        UpdateState.Ready => Restart(),
        _ => Task.CompletedTask,
    };

    private async Task Check()
    {
        State = UpdateState.Checking;
        Changed();
        try
        {
            // Resumes on the caller's thread (the UI thread's context).
            Version = await updater.CheckAsync().ConfigureAwait(true);
            State = Version is null ? UpdateState.UpToDate : UpdateState.Available;
        }
        catch (Exception e)
        {
            Fail($"Couldn't check for updates: {Reason(e)}", "check");
        }
        Changed();
    }

    private async Task Download()
    {
        State = UpdateState.Downloading;
        percent = 0;
        Changed();
        // Progress arrives on the updater's threads: each new percent is posted to the thread that
        // pressed the button (the UI thread's context). Without a context it shows at the end.
        var ui = SynchronizationContext.Current;
        try
        {
            await updater.DownloadAsync(p => ui?.Post(_ => Progressed(p), null)).ConfigureAwait(true);
            percent = 100;
            State = UpdateState.Ready;
        }
        catch (Exception e)
        {
            Fail($"Couldn't download Inkwell {Version}: {Reason(e)}", "download");
        }
        Changed();
    }

    /// <summary>UI thread. A download's progress, while it is the one shown.</summary>
    private void Progressed(int p)
    {
        p = Math.Clamp(p, 0, 100);
        if (State == UpdateState.Downloading && p > percent)
        {
            percent = p;
            Changed();
        }
    }

    private Task Restart()
    {
        try
        {
            updater.RestartToUpdate();
        }
        catch (Exception e)
        {
            // The installer did not start (missing, or blocked: it is not code-signed yet), so the
            // app is still running and the row says why.
            Fail($"Couldn't restart to install Inkwell {Version}: {Reason(e)}", "restart");
            Changed();
        }
        return Task.CompletedTask;
    }

    private void Fail(string line, string step)
    {
        // The kind only in the log, as elsewhere: the reason is on screen.
        log.Write($"update {step} failed");
        failure = line;
        State = UpdateState.Failed;
    }

    /// <summary>The failure's own words, one line, ending in a full stop.</summary>
    private static string Reason(Exception e)
    {
        var text = string.Join(' ', (e.Message ?? e.GetType().Name).Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));
        if (text.Length == 0)
        {
            text = e.GetType().Name;
        }
        return text.EndsWith('.') ? text : text + ".";
    }
}
