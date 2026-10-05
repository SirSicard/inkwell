// Whether the main window is hidden behind others, as the Mac reads its window's occlusion state.
// Windows tells a desktop app nothing of the kind, so this follows the events that change what
// covers what: another window brought to the front, minimised or restored, or moved or resized by
// the user; while it is covered, also a window closed, hidden or cloaked (WinEvent hooks, out of
// context, on the UI thread: no polling; WindowCoverEvents chooses them). A covering window moved or
// maximised by code alone is not heard (that would be a wake for every move of the mouse): it
// stays covered until the next of these. At each one, the windows above this one in the z-order
// are measured against it (WindowFrame.CoveredBy); a burst of window events once it settles.
// Windows that show nothing are left out: hidden, minimised, cloaked (another virtual desktop, a
// suspended app) or click-through overlays; so are this process's own (a dialog or menu over it).
// Hidden in the tray, it hooks nothing, so nothing on the desktop wakes the app.
//
// A covered window that is uncovered raises Uncovered: the orb at rest goes to a new spot, as on
// the Mac when the window comes back into view. Covered also feeds WindowPresence, so what redraws
// on a clock waits while nobody can see it.
using System.Diagnostics;
using System.Runtime.InteropServices;
using Inkwell.Core.Screens;

namespace Inkwell.Core;

public sealed partial class WindowCover : IDisposable
{
    private const uint WinEventOutOfContext = 0;
    private const uint GaRoot = 2;
    private const int ObjIdWindow = 0;
    private const int ChildIdSelf = 0;
    private const uint GwHwndPrev = 3;
    private const int GwlExStyle = -20;
    private const int WsExTransparent = 0x20;
    private const uint DwmwaExtendedFrameBounds = 9;
    private const uint DwmwaCloaked = 14;

    private readonly nint window;
    private readonly ScreenLog log;
    /// <summary>The hooks made, by their event range (WindowCoverEvents.Hooks).</summary>
    private readonly Dictionary<(uint Min, uint Max), nint> hooks = [];
    /// <summary>Held, so the hook's delegate is never collected while Windows calls it.</summary>
    private readonly WinEventProc proc;
    /// <summary>The UI thread, where a settled burst is measured (null: each event measures at once).</summary>
    private readonly SynchronizationContext? ui;
    /// <summary>Restarted by each window event; when it fires, one measure on the UI thread.</summary>
    private readonly Timer settle;
    /// <summary>When the burst of window events now settling began (Stopwatch), or 0.</summary>
    private long burstBegan;
    private bool shown;
    private bool failureSaid;
    private bool measureFailureSaid;
    private bool postFailureSaid;
    private bool disposed;

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
    /// <param name="log">Where a hook that could not be made is said (the window then counts as uncovered).</param>
    public WindowCover(nint window, ScreenLog? log = null)
    {
        this.window = window;
        this.log = log ?? ScreenLog.System;
        proc = OnEvent;
        ui = SynchronizationContext.Current;
        settle = new Timer(Settled);
        shown = IsWindowVisible(window);
        Covered = Measure();
        Hook();
    }

    /// <summary>The window was shown (true) or hidden, to the tray. Hidden, nothing is hooked. UI thread.</summary>
    public void WindowShown(bool now)
    {
        if (now == shown || disposed)
        {
            return;
        }
        shown = now;
        if (now)
        {
            Hook();
            MeasureNow();
        }
        else
        {
            _ = settle.Change(Timeout.Infinite, Timeout.Infinite);
            burstBegan = 0;
            Hook();
        }
    }

    /// <summary>Hooks what WindowCoverEvents asks for now, and lets go of the rest.</summary>
    private void Hook()
    {
        var wanted = WindowCoverEvents.Hooks(shown, Covered);
        foreach (var (range, hook) in hooks.Where(h => !wanted.Contains(h.Key)).ToList())
        {
            _ = UnhookWinEvent(hook);
            _ = hooks.Remove(range);
        }
        var callback = Marshal.GetFunctionPointerForDelegate(proc);
        foreach (var range in wanted.Where(r => !hooks.ContainsKey(r)))
        {
            var hook = SetWinEventHook(range.Min, range.Max, 0, callback, 0, 0, WinEventOutOfContext);
            if (hook != 0)
            {
                hooks[range] = hook;
            }
            else if (!failureSaid)
            {
                // Said once; the next change of cover tries again.
                failureSaid = true;
                log.Write("couldn't follow all of what covers the window; it may count as uncovered, or stay covered until another window comes to the front");
            }
        }
    }

    /// <summary>Whether other windows hide all of it now.</summary>
    public bool Covered { get; private set; }

    /// <summary>Covered or uncovered. UI thread.</summary>
    public event Action<bool>? Changed;

    /// <summary>It was covered, and now is not. UI thread.</summary>
    public event Action? Uncovered;

    private void OnEvent(nint h, uint eventType, nint hwnd, int idObject, int idChild, uint thread, uint time)
    {
        // Only whole windows, top-level for the window events (EVENT_OBJECT_*): a caret, the
        // pointer, a menu item or a child window is not a window covering this one. Checked before
        // anything else: the window events come for those too. A window already gone (closed: the
        // event comes after) can't be asked, and counts.
        var topLevel = eventType >= 0x8000 && idObject == ObjIdWindow && idChild == ChildIdSelf
            && (!IsWindow(hwnd) || GetAncestor(hwnd, GaRoot) == hwnd);
        if (disposed || !WindowCoverEvents.Measures(eventType, idObject, idChild, topLevel))
        {
            return;
        }
        if (WindowCoverEvents.Settles(eventType) && ui is not null)
        {
            // Measured once, when the burst (an app closing its windows) has settled, or at most
            // MaxWait after it began.
            var now = Stopwatch.GetTimestamp();
            if (burstBegan == 0)
            {
                burstBegan = now;
            }
            _ = settle.Change(WindowCoverEvents.SettleDelay(Stopwatch.GetElapsedTime(burstBegan, now)), Timeout.InfiniteTimeSpan);
            return;
        }
        MeasureNow();
    }

    /// <summary>The timer's thread: the burst has settled, so one measure, on the UI thread.</summary>
    private void Settled(object? state)
    {
        try
        {
            ui?.Post(_ =>
            {
                burstBegan = 0;
                MeasureNow();
            }, null);
        }
        catch (Exception e)
        {
            // Never out of the timer's thread (the UI thread may be shutting down): said once, by type.
            if (!postFailureSaid)
            {
                postFailureSaid = true;
                log.Write($"couldn't measure what covers the window after a burst ({e.GetType().Name})");
            }
        }
    }

    /// <summary>Measures now, and says it if covered changed. UI thread.</summary>
    private void MeasureNow()
    {
        // Hidden, nothing measures, a measure already posted included.
        if (disposed || !shown)
        {
            return;
        }
        try
        {
            var now = Measure();
            if (now == Covered)
            {
                return;
            }
            Covered = now;
            // Covered, it hears what uncovers it too; uncovered, only what can cover it.
            Hook();
            // A cover gone between the measure and its hooks would never be heard: measured once
            // more, it is still uncovered and nothing changed.
            if (now && !Measure())
            {
                Covered = false;
                Hook();
                return;
            }
            Changed?.Invoke(now);
            if (!now)
            {
                Uncovered?.Invoke();
            }
        }
        catch (Exception e)
        {
            // Never into Windows' callback: named in the log once, and the next event measures again.
            if (!measureFailureSaid)
            {
                measureFailureSaid = true;
                log.Write($"couldn't measure what covers the window: {e.GetType().Name}");
            }
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
        disposed = true;
        settle.Dispose();
        foreach (var hook in hooks.Values)
        {
            _ = UnhookWinEvent(hook);
        }
        hooks.Clear();
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
    private static partial nint GetAncestor(nint hwnd, uint flags);

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool IsWindow(nint hwnd);

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
