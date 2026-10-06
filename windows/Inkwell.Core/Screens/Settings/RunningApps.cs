// The mode editor's "Running now" (IRunningApps): the apps with a visible top-level window, each by
// its executable's file name, as the core matches the app in front. Each window's process is opened
// with limited rights only (PROCESS_QUERY_LIMITED_INFORMATION) for its image's path, so an elevated
// app is listed too. Read when the menu opens, never on a timer.
//
// Left out: Inkwell itself; windows that are hidden, cloaked (another virtual desktop's, a
// suspended Store app's), owned (dialogs) or tool windows; and Store apps' host,
// ApplicationFrameHost.exe, which every Store app's window shows as (a mode naming it would match
// all of them). A Store app with its own process (Windows Terminal, say) is listed by that. Paths
// never reach a log. Here, beside WindowCover, as its Win32 calls need the core's unsafe code.
using System.Runtime.InteropServices;

namespace Inkwell.Core.Screens;

public sealed partial class RunningApps(IAppDirectory names) : IRunningApps
{
    /// <summary>Processes whose windows are the shell's, or every Store app's host: never a mode's.</summary>
    private static readonly HashSet<string> Hidden = new(StringComparer.OrdinalIgnoreCase)
    {
        ModesModel.InkwellExe,
        "applicationframehost.exe",
        "shellexperiencehost.exe",
        "startmenuexperiencehost.exe",
        "searchhost.exe",
        "textinputhost.exe",
        "lockapp.exe",
    };

    private const uint GwHwndNext = 2;
    private const uint GwOwner = 4;
    private const int GwlExStyle = -20;
    private const int WsExToolWindow = 0x80;
    private const uint DwmwaCloaked = 14;
    private const uint ProcessQueryLimitedInformation = 0x1000;

    public IReadOnlyList<PickedApp> Running()
    {
        var own = (uint)Environment.ProcessId;
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var found = new List<PickedApp>();
        // Top-level windows, front to back: GetTopWindow then GW_HWNDNEXT (no callback to marshal).
        for (var window = GetTopWindow(0); window != 0; window = GetWindow(window, GwHwndNext))
        {
            if (!IsWindowVisible(window) || GetWindow(window, GwOwner) != 0 || GetWindowTextLengthW(window) == 0
                || (GetWindowLongW(window, GwlExStyle) & WsExToolWindow) != 0
                || (DwmGetWindowAttribute(window, DwmwaCloaked, out int cloaked, sizeof(int)) >= 0 && cloaked != 0))
            {
                continue;
            }
            _ = GetWindowThreadProcessId(window, out var process);
            if (process == own || ImagePath(process) is not string path)
            {
                continue;
            }
            var exe = Path.GetFileName(path).ToLowerInvariant();
            if (!AppIdentity.IsExe(exe) || Hidden.Contains(exe) || !seen.Add(exe))
            {
                continue;
            }
            var label = AppIdentity.Label(exe, names);
            found.Add(new PickedApp(exe, label.Name, label.IconPath ?? path));
        }
        return [.. found.OrderBy(a => a.Name, StringComparer.CurrentCultureIgnoreCase)];
    }

    /// <summary>A process's image path, asked with limited rights; null when it can't be read.</summary>
    private static string? ImagePath(uint process)
    {
        var handle = OpenProcess(ProcessQueryLimitedInformation, false, process);
        if (handle == 0)
        {
            return null;
        }
        try
        {
            var buffer = new char[1024];
            var size = (uint)buffer.Length;
            return QueryFullProcessImageNameW(handle, 0, buffer, ref size) ? new string(buffer, 0, (int)size) : null;
        }
        finally
        {
            _ = CloseHandle(handle);
        }
    }

    [LibraryImport("user32.dll")]
    private static partial nint GetTopWindow(nint parent);

    [LibraryImport("user32.dll")]
    private static partial nint GetWindow(nint hwnd, uint cmd);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool IsWindowVisible(nint hwnd);

    [LibraryImport("user32.dll")]
    private static partial int GetWindowTextLengthW(nint hwnd);

    [LibraryImport("user32.dll")]
    private static partial int GetWindowLongW(nint hwnd, int index);

    [LibraryImport("user32.dll")]
    private static partial uint GetWindowThreadProcessId(nint hwnd, out uint process);

    [LibraryImport("dwmapi.dll")]
    private static partial int DwmGetWindowAttribute(nint hwnd, uint attribute, out int value, int size);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    private static partial nint OpenProcess(uint access, [MarshalAs(UnmanagedType.Bool)] bool inherit, uint process);

    [LibraryImport("kernel32.dll", SetLastError = true, StringMarshalling = StringMarshalling.Utf16)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool QueryFullProcessImageNameW(nint process, uint flags, [Out] char[] name, ref uint size);

    [LibraryImport("kernel32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CloseHandle(nint handle);
}
