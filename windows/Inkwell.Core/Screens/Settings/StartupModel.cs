// Start with Windows (Settings > General and the tray menu): the Windows counterpart of the Mac's
// Open at Login. The app's entry is the installed copy's (WindowsStartup: a value under the user's
// Run key naming the installer's launcher), so only the installed app offers it; a copy run from a
// folder shows the switch off and says why. A switch that could not be set says "Couldn't ..."
// and shows what Windows holds. Nothing is read or written until the switch shows or is pressed.
namespace Inkwell.Core.Screens;

/// <summary>Where the app starts with Windows from.</summary>
public interface IStartupEntry
{
    /// <summary>Why this copy cannot start with Windows, or null when it can (the installed app).</summary>
    string? Unavailable { get; }

    /// <summary>Whether Windows starts this copy at sign-in. Throws when that cannot be read.</summary>
    bool IsOn();

    /// <summary>Starts this copy at sign-in, or stops doing so. Throws when that cannot be written.</summary>
    void Change(bool enabled);
}

/// <summary>The switch. UI thread only, like every screen model.</summary>
public sealed class StartupModel(IStartupEntry? entry, ScreenLog? log = null) : ObservableModel
{
    public const string Title = "Start with Windows";
    public const string Caption = "Inkwell opens in the notification area when you sign in";
    public const string NotInstalledText = "Only the installed app can start with Windows.";

    private readonly ScreenLog log = log ?? ScreenLog.System;

    /// <summary>Whether the switch can be used.</summary>
    public bool Available => entry is not null && entry.Unavailable is null;

    /// <summary>Why the switch cannot be used, or null.</summary>
    public string? Unavailable => entry is null ? NotInstalledText : entry.Unavailable;

    /// <summary>Whether Windows starts Inkwell at sign-in (as last read).</summary>
    public bool IsOn { get; private set; }

    /// <summary>Why the last read or change failed, or null.</summary>
    public string? Failure { get; private set; }

    /// <summary>Reads what Windows holds (when the switch shows).</summary>
    public void Refresh()
    {
        if (!Available)
        {
            return;
        }
        try
        {
            var on = entry!.IsOn();
            if (on == IsOn && Failure is null)
            {
                return;
            }
            IsOn = on;
            Failure = null;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or System.Security.SecurityException)
        {
            log.Write($"whether Inkwell starts with Windows could not be read ({e.GetType().Name})");
            Failure = "Couldn't read whether Inkwell starts with Windows.";
        }
        Changed();
    }

    /// <summary>The switch.</summary>
    public void SetOn(bool on)
    {
        if (!Available || (on == IsOn && Failure is null))
        {
            return;
        }
        try
        {
            entry!.Change(on);
            IsOn = on;
            Failure = null;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or System.Security.SecurityException)
        {
            log.Write($"starting with Windows could not be {(on ? "turned on" : "turned off")} ({e.GetType().Name})");
            Failure = on ? "Couldn't make Inkwell start with Windows." : "Couldn't stop Inkwell starting with Windows.";
        }
        Changed();
    }
}
