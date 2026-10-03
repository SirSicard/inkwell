// Whether the main window is hidden behind others, as the Mac reads its window's occlusion state.
// Windows tells a desktop app nothing of the kind, so this follows the events that change what
// covers what: another window brought to the front, minimised or restored, or moved or resized by
// the user (a WinEvent hook, out of context, on the UI thread: no polling). At each one, the windows
// above this one in the z-order are measured against it (WindowFrame.CoveredBy). Windows that show
// nothing are left out: hidden, minimised, cloaked (another virtual desktop, a suspended app) or
// click-through overlays; so are this process's own (a dialog or menu over it).
//
// A covered window that is uncovered raises Uncovered: the orb at rest goes to a new spot, as on
// the Mac when the window comes back into view. Covered also feeds WindowPresence, so what redraws
// on a clock waits while nobody can see it.
using System.Runtime.InteropServices;
using Inkwell.Core.Screens;

namespace Inkwell.Core;

public sealed partial class WindowCover : IDisposable
{
    private const uint EventSystemForeground = 0x0003;
    private const uint EventSystemMinimizeEnd = 0x0017;
    private const uint WinEventOutOfContext = 0;
    private const int ObjIdWindow = 0;
    private const uint GwHwndPrev = 3;
    private const int GwlExStyle = -20;
    private const int WsExTransparent = 0x20;
    private const uint DwmwaExtendedFrameBounds = 9;
    private const uint DwmwaCloaked = 14;

    private readonly nint window;
    private readonly nint hook;
    /// <summary>Held, so the hook's delegate is never collected while Windows calls it.</summary>
    private readonly WinEventProc proc;

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate void WinEventProc(nint hook, uint eventType, nint hwnd, int idObject, int idChild, uint thread, uint time);

    [StructLayout(LayoutKind.Sequential)]
    // 16 bytes: four ints, as Win32's RECT.
    private struct Rect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    /// <summary>UI thread: the hook's events arrive on it.</summary>
    public WindowCover(nint window)
    {
        this.window = window;
        proc = OnEvent;
        // EVENT_SYSTEM_FOREGROUND to EVENT_SYSTEM_MINIMIZEEND: the front window changes, a window
        // is moved or resized (MOVESIZEEND), minimised or restored.
        hook = SetWinEventHook(EventSystemForeground, EventSystemMinimizeEnd, 0, Marshal.GetFunctionPointerForDelegate(proc), 0, 0, WinEventOutOfContext);
        Covered = Measure();
    }

    /// <summary>Whether other windows hide all of it now.</summary>
    public bool Covered { get; private set; }

    /// <summary>Covered or uncovered. UI thread.</summary>
    public event Action<bool>? Changed;

    /// <summary>It was covered, and now is not. UI thread.</summary>
    public event Action? Uncovered;

    private void OnEvent(nint h, uint eventType, nint hwnd, int idObject, int idChild, uint thread, uint time)
    {
        // Only whole windows: a caret or a menu item is not a window moving.
        if (idObject != ObjIdWindow)
        {
            return;
        }
        var now = Measure();
        if (now == Covered)
        {
            return;
        }
        Covered = now;
        Changed?.Invoke(now);
        if (!now)
        {
            Uncovered?.Invoke();
        }
    }

    /// <summary>Whether the windows above it hide all of it (not when it is hidden or minimised itself: that is not being covered).</summary>
    private bool Measure()
    {
        if (!IsWindowVisible(window) || IsIconic(window) || Frame(window) is not { } mine)
        {
            return false;
        }
        _ = GetWindowThreadProcessId(window, out var process);
        var above = new List<WindowFrame>();
        // Up the z-order from this window: everything drawn over it.
        for (var w = GetWindow(window, GwHwndPrev); w != 0; w = GetWindow(w, GwHwndPrev))
        {
            if (Shows(w, process) && Frame(w) is { } frame)
            {
                above.Add(frame);
            }
        }
        return mine.CoveredBy(above);
    }

    /// <summary>A window that shows something over this one.</summary>
    private static bool Shows(nint w, uint ownProcess)
    {
        if (!IsWindowVisible(w) || IsIconic(w))
        {
            return false;
        }
        _ = GetWindowThreadProcessId(w, out var process);
        if (process == ownProcess)
        {
            return false;
        }
        // Click-through overlays draw over everything and hide nothing.
        if ((GetWindowLongW(w, GwlExStyle) & WsExTransparent) != 0)
        {
            return false;
        }
        return DwmGetWindowAttribute(w, DwmwaCloaked, out int cloaked, sizeof(int)) < 0 || cloaked == 0;
    }

    /// <summary>What a window shows on screen: DWM's frame, without the invisible resize borders.</summary>
    private static WindowFrame? Frame(nint w)
    {
        if (DwmGetWindowAttribute(w, DwmwaExtendedFrameBounds, out Rect r, 16) < 0 && !GetWindowRect(w, out r))
        {
            return null;
        }
        return new WindowFrame(r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top);
    }

    public void Dispose()
    {
        if (hook != 0)
        {
            _ = UnhookWinEvent(hook);
        }
    }

    [LibraryImport("user32.dll")]
    private static partial nint SetWinEventHook(uint eventMin, uint eventMax, nint module, nint callback, uint process, uint thread, uint flags);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool UnhookWinEvent(nint hook);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool IsWindowVisible(nint hwnd);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool IsIconic(nint hwnd);

    [LibraryImport("user32.dll")]
    private static partial nint GetWindow(nint hwnd, uint cmd);

    [LibraryImport("user32.dll")]
    private static partial uint GetWindowThreadProcessId(nint hwnd, out uint process);

    [LibraryImport("user32.dll")]
    private static partial int GetWindowLongW(nint hwnd, int index);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetWindowRect(nint hwnd, out Rect rect);

    [LibraryImport("dwmapi.dll")]
    private static partial int DwmGetWindowAttribute(nint hwnd, uint attribute, out Rect value, int size);

    [LibraryImport("dwmapi.dll")]
    private static partial int DwmGetWindowAttribute(nint hwnd, uint attribute, out int value, int size);
}
