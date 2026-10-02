// Start with Windows: a value named "Inkwell" under the user's Run key
// (HKCU\Software\Microsoft\Windows\CurrentVersion\Run), naming the installed app's launcher, which
// Velopack keeps at the install folder's root (it starts the current version, so the entry survives
// updates). Only the installed app has one: a copy run from a folder (a development build, a
// publish not installed) says so and writes nothing. Nothing is read or written until Settings or
// the tray asks; uninstalling with Velopack leaves the value, which then names a missing file and
// does nothing (Settings shows it off once read).
using Inkwell.Core.Screens;
using Microsoft.Win32;
using Velopack.Locators;

namespace Inkwell;

internal sealed class WindowsStartup : IStartupEntry
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "Inkwell";

    /// <summary>The installed launcher's path, or null when this copy is not installed.</summary>
    private readonly string? launcher = Launcher();

    public string? Unavailable => launcher is null ? StartupModel.NotInstalledText : null;

    public bool IsOn()
    {
        using var run = Registry.CurrentUser.OpenSubKey(RunKey, writable: false);
        return run?.GetValue(ValueName) is string command && launcher is not null
            && string.Equals(command.Trim('"'), launcher, StringComparison.OrdinalIgnoreCase);
    }

    public void Change(bool enabled)
    {
        if (launcher is null)
        {
            throw new InvalidOperationException(StartupModel.NotInstalledText);
        }
        using var run = Registry.CurrentUser.CreateSubKey(RunKey, writable: true)
            ?? throw new UnauthorizedAccessException("the Run key could not be opened");
        if (enabled)
        {
            run.SetValue(ValueName, $"\"{launcher}\"", RegistryValueKind.String);
        }
        else
        {
            run.DeleteValue(ValueName, throwOnMissingValue: false);
        }
    }

    /// <summary>The Velopack launcher of the installed app (its root's copy of this exe), or null when not installed.</summary>
    private static string? Launcher()
    {
        try
        {
            if (!VelopackLocator.IsCurrentSet)
            {
                return null;
            }
            var locator = VelopackLocator.Current;
            if (locator.IsPortable || locator.RootAppDir is not { } root || locator.ThisExeRelativePath is not { } exe
                || locator.CurrentlyInstalledVersion is null)
            {
                return null;
            }
            var stub = Path.Combine(root, Path.GetFileName(exe));
            if (File.Exists(stub))
            {
                return stub;
            }
            return locator.AppContentDir is { } content && File.Exists(Path.Combine(content, exe)) ? Path.Combine(content, exe) : null;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or InvalidOperationException)
        {
            ScreenLog.System.Write($"where Inkwell is installed could not be read ({e.GetType().Name})");
            return null;
        }
    }
}
