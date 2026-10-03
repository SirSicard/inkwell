// The main window's messages the WinUI window does not hand on: the taskbar's (its button made,
// a thumbnail button clicked) and the session's (the screen locked or unlocked, the display on or
// off). A subclass of the window's procedure (SetWindowSubclass) on the UI thread; it raises an
// event for each and passes every message on. The session's are asked for here
// (WTSRegisterSessionNotification, RegisterPowerSettingNotification for the session's display),
// so the live icon's pulse stops while nobody can see the screen: nothing polls.
using System.Runtime.InteropServices;
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

public sealed unsafe partial class WindowHook : IDisposable
{
    private const uint WmCommand = 0x0111;
    private const uint WmPowerBroadcast = 0x0218;
    private const uint WmWtsSessionChange = 0x02B1;
    private const uint PbtPowerSettingChange = 0x8013;
    private const int WtsSessionLock = 7;
    private const int WtsSessionUnlock = 8;
    private const int ThbnClicked = 0x1800;

    /// <summary>GUID_SESSION_DISPLAY_STATUS: this session's display, off (0), on (1) or dimmed (2).</summary>
    private static readonly Guid SessionDisplayStatus = new("2B84C20E-AD23-4DDF-93DB-05FFBD7EFCA5");

    private readonly HWND window;
    private readonly GCHandle self;
    private readonly uint taskbarCreated;
    private readonly HPOWERNOTIFY display;
    private bool disposed;

    /// <summary>UI thread: subclasses <paramref name="window"/> until disposed.</summary>
    public WindowHook(nint window)
    {
        this.window = (HWND)window;
        self = GCHandle.Alloc(this);
        fixed (char* name = "TaskbarButtonCreated")
        {
            taskbarCreated = RegisterWindowMessageW(name);
        }
        if (!SetWindowSubclass(this.window, &Procedure, 1, (nuint)(nint)GCHandle.ToIntPtr(self)))
        {
            self.Free();
            throw new InkRendererException($"couldn't watch the window's messages (error {GetLastError()})");
        }
        // Best effort: without them the pulse runs while locked, which it never needs to.
        if (!WTSRegisterSessionNotification(window, 0))
        {
            InkLog.Write("couldn't hear the screen lock: the live icon's pulse runs while it is locked");
        }
        var guid = SessionDisplayStatus;
        display = RegisterPowerSettingNotification((HANDLE)this.window.Value, &guid, 0);
        if (display == HPOWERNOTIFY.NULL)
        {
            InkLog.Write("couldn't hear the display go off: the live icon's pulse runs while it is off");
        }
    }

    /// <summary>The window's taskbar button was made (at start, and again when Explorer restarts).</summary>
    public event Action? TaskbarButtonCreated;

    /// <summary>A thumbnail toolbar button was clicked: its id.</summary>
    public event Action<int>? ThumbnailClicked;

    /// <summary>The screen was locked (true) or unlocked.</summary>
    public event Action<bool>? Locked;

    /// <summary>This session's display went off (false) or on again, dimmed counting as on.</summary>
    public event Action<bool>? DisplayOn;

    [UnmanagedCallersOnly]
    private static LRESULT Procedure(HWND hwnd, uint message, WPARAM wParam, LPARAM lParam, nuint id, nuint data)
    {
        if (GCHandle.FromIntPtr((nint)data).Target is WindowHook hook)
        {
            try
            {
                hook.Handle(message, wParam, lParam);
            }
            catch (Exception e)
            {
                // Never into the window's procedure: named, and the message goes on.
                InkLog.Write($"the window's message hook failed: {e.GetType().Name}: {e.Message}");
            }
        }
        return DefSubclassProc(hwnd, message, wParam, lParam);
    }

    private void Handle(uint message, WPARAM wParam, LPARAM lParam)
    {
        if (message == taskbarCreated)
        {
            TaskbarButtonCreated?.Invoke();
        }
        else if (message == WmCommand && (int)((nuint)wParam >> 16 & 0xFFFF) == ThbnClicked)
        {
            ThumbnailClicked?.Invoke((int)((nuint)wParam & 0xFFFF));
        }
        else if (message == WmWtsSessionChange)
        {
            var reason = (int)(nuint)wParam;
            if (reason == WtsSessionLock)
            {
                Locked?.Invoke(true);
            }
            else if (reason == WtsSessionUnlock)
            {
                Locked?.Invoke(false);
            }
        }
        else if (message == WmPowerBroadcast && (nuint)wParam == PbtPowerSettingChange && lParam != 0)
        {
            var setting = (POWERBROADCAST_SETTING*)(nint)lParam;
            if (setting->PowerSetting == SessionDisplayStatus && setting->DataLength >= sizeof(uint))
            {
                DisplayOn?.Invoke(*(uint*)&setting->Data != 0);
            }
        }
    }

    [LibraryImport("wtsapi32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool WTSRegisterSessionNotification(nint window, uint flags);

    [LibraryImport("wtsapi32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool WTSUnRegisterSessionNotification(nint window);

    public void Dispose()
    {
        if (disposed)
        {
            return;
        }
        disposed = true;
        _ = RemoveWindowSubclass(window, &Procedure, 1);
        _ = WTSUnRegisterSessionNotification((nint)window.Value);
        if (display != HPOWERNOTIFY.NULL)
        {
            _ = UnregisterPowerSettingNotification(display);
        }
        self.Free();
    }
}
