// The installed apps, as Settings > Modes names them (IAppDirectory): an executable's file name to
// the app's name and the file its icon comes from. The Mac asks Launch Services; Windows has no
// one place, so this reads, in order:
//
//   1. App Paths (HKCU, then HKLM): the key named after the exe holds its full path.
//   2. The Uninstall entries (HKCU, HKLM and HKLM's 32-bit view): an entry whose DisplayIcon is an
//      .exe of that name, with the name the app installed under (DisplayName).
//
// The name is the uninstall entry's, else the exe's own FileDescription or ProductName, else the
// stem (InstalledAppValues.Name). Start-menu shortcuts are not read: their targets need
// IShellLink, a COM interface the app does not marshal, so an app found only there
// reads as a well-known name or its stem. Packaged (Store) apps are not listed either.
//
// Registry only, no files opened but the exe's version resource. The Uninstall entries are read
// once, off the UI thread when Warm() is called at start, else on the first lookup; each answer
// (found or not) is cached for the app's life. Paths never reach a log.
using System.Collections.Concurrent;
using System.Diagnostics;
using Inkwell.Core.Screens;
using Microsoft.Win32;

namespace Inkwell.Screens;

public sealed class InstalledApps : IAppDirectory
{
    private const string AppPaths = @"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths";
    private const string Uninstall = @"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

    private readonly ConcurrentDictionary<string, InstalledApp?> answers = new(StringComparer.OrdinalIgnoreCase);
    private readonly Lazy<Dictionary<string, (string Path, string? Name)>> uninstallIndex = new(ReadUninstall, LazyThreadSafetyMode.ExecutionAndPublication);

    public static InstalledApps Shared { get; } = new();

    /// <summary>Reads the Uninstall entries off the UI thread, so the first Settings > Modes does not wait for them.</summary>
    public void Warm() => _ = Task.Run(() => uninstallIndex.Value);

    public InstalledApp? App(string exe)
    {
        ArgumentNullException.ThrowIfNull(exe);
        var name = exe.Trim();
        if (!AppIdentity.IsExe(name) || name.Contains('\\', StringComparison.Ordinal) || name.Contains('/', StringComparison.Ordinal))
        {
            return null;
        }
        return answers.GetOrAdd(name, Find);
    }

    private InstalledApp? Find(string exe)
    {
        string? path = null;
        string? displayName = null;
        foreach (var hive in new[] { Registry.CurrentUser, Registry.LocalMachine })
        {
            path = Read(() =>
            {
                using var key = hive.OpenSubKey($@"{AppPaths}\{exe}");
                return InstalledAppValues.ExePath(key?.GetValue(null) as string);
            });
            if (path is not null && File.Exists(path))
            {
                break;
            }
            path = null;
        }
        if (uninstallIndex.Value.TryGetValue(exe, out var entry))
        {
            path ??= entry.Path;
            displayName = entry.Name;
        }
        if (path is null || !File.Exists(path))
        {
            return null;
        }
        var version = Read(() => FileVersionInfo.GetVersionInfo(path));
        return new InstalledApp(InstalledAppValues.Name(exe, displayName, version?.FileDescription, version?.ProductName), path);
    }

    /// <summary>Every Uninstall entry whose DisplayIcon is an .exe, by that exe's file name (the first entry wins).</summary>
    private static Dictionary<string, (string Path, string? Name)> ReadUninstall()
    {
        var index = new Dictionary<string, (string Path, string? Name)>(StringComparer.OrdinalIgnoreCase);
        var roots = new (RegistryHive Hive, RegistryView View)[]
        {
            (RegistryHive.CurrentUser, RegistryView.Default),
            (RegistryHive.LocalMachine, RegistryView.Registry64),
            (RegistryHive.LocalMachine, RegistryView.Registry32),
        };
        foreach (var (hive, view) in roots)
        {
            Read(() =>
            {
                using var root = RegistryKey.OpenBaseKey(hive, view);
                using var entries = root.OpenSubKey(Uninstall);
                if (entries is null)
                {
                    return 0;
                }
                foreach (var id in entries.GetSubKeyNames())
                {
                    Read(() =>
                    {
                        using var app = entries.OpenSubKey(id);
                        if (InstalledAppValues.ExePath(app?.GetValue("DisplayIcon") as string) is string path)
                        {
                            index.TryAdd(InstalledAppValues.FileName(path), (path, app?.GetValue("DisplayName") as string));
                        }
                        return 0;
                    });
                }
                return 0;
            });
        }
        return index;
    }

    /// <summary>A registry or file read that may be refused or gone: nothing, then, never a crash.</summary>
    private static T? Read<T>(Func<T?> read)
    {
        try
        {
            return read();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or System.Security.SecurityException or ArgumentException)
        {
            return default;
        }
    }
}
