// What the Windows app directory (Inkwell/Screens/Settings/InstalledApps.cs) reads out of the
// registry, parsed without touching it, so it can be tested anywhere: an App Paths default value,
// an Uninstall key's DisplayIcon, and which name an app goes by.
namespace Inkwell.Core.Screens;

public static class InstalledAppValues
{
    /// <summary>
    /// The .exe a registry value names, or null: an App Paths default (<c>"C:\A\a.exe"</c>, quoted
    /// or not) or a DisplayIcon (<c>C:\A\a.exe,0</c>, possibly quoted, possibly with an icon index).
    /// Only an absolute path to an .exe counts: an .ico, a DLL or an installer's icon names no app.
    /// </summary>
    public static string? ExePath(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }
        var text = value.Trim();
        if (text.StartsWith('"'))
        {
            var close = text.IndexOf('"', 1);
            text = close > 0 ? text[1..close] : text[1..];
        }
        else
        {
            // An icon index after the path: "C:\A\a.exe,0" or "C:\A\a.exe,-101".
            var comma = text.LastIndexOf(',');
            if (comma > 0 && int.TryParse(text.AsSpan(comma + 1).Trim(), out _))
            {
                text = text[..comma];
            }
        }
        text = text.Trim();
        if (!text.EndsWith(".exe", StringComparison.OrdinalIgnoreCase) || !IsAbsoluteWindowsPath(text))
        {
            return null;
        }
        return text;
    }

    /// <summary>The executable's file name in a Windows path: <c>C:\A\Slack.exe</c> -> <c>Slack.exe</c>.</summary>
    public static string FileName(string path)
    {
        ArgumentNullException.ThrowIfNull(path);
        var cut = path.LastIndexOfAny(['\\', '/']);
        return cut < 0 ? path : path[(cut + 1)..];
    }

    /// <summary>
    /// The name an installed app goes by: the uninstall entry's, else the file's description, else
    /// its product name, else the file name's stem. Blank names are skipped; the result is never an
    /// exe name.
    /// </summary>
    public static string Name(string exe, string? displayName, string? fileDescription, string? productName)
    {
        ArgumentNullException.ThrowIfNull(exe);
        foreach (var candidate in new[] { displayName, fileDescription, productName })
        {
            var name = candidate?.Trim();
            if (!string.IsNullOrEmpty(name) && !name.EndsWith(".exe", StringComparison.OrdinalIgnoreCase))
            {
                return name;
            }
        }
        return AppIdentity.Label(exe, NoInstalledApps.Instance).Name;
    }

    private static bool IsAbsoluteWindowsPath(string path) =>
        (path.Length > 2 && char.IsAsciiLetter(path[0]) && path[1] == ':' && path[2] is '\\' or '/')
        || path.StartsWith(@"\\", StringComparison.Ordinal);
}
