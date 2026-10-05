// The main window's message hook (WindowHook), on a hidden window of the test's own: the messages it
// hears are sent to it as Windows would, and a handler that fails is logged by its type only (an
// exception's message can carry what the log must never hold), while the message goes on.
using TerraFX.Interop.Windows;
using Xunit;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink.Tests;

public sealed unsafe class WindowHookTests
{
    /// <summary>A hidden top-level window on this thread (never shown).</summary>
    private static HWND Window()
    {
        fixed (char* kind = "STATIC")
        {
            var window = CreateWindowExW(0, kind, null, 0, 0, 0, 0, 0, HWND.NULL, HMENU.NULL, HINSTANCE.NULL, null);
            Assert.NotEqual(HWND.NULL, window);
            return window;
        }
    }

    private static uint TaskbarButtonCreated()
    {
        fixed (char* name = "TaskbarButtonCreated")
        {
            return RegisterWindowMessageW(name);
        }
    }

    /// <summary>
    /// The session's changes as Windows sends them (WM_WTSSESSION_CHANGE): a disconnect and a
    /// connect, at the console (fast user switching) or remote, and the lock; anything else (a
    /// logon, say) is not heard.
    /// </summary>
    [Fact]
    public void TheSessionsChangesAreHeard()
    {
        const uint WmWtsSessionChange = 0x02B1;
        var window = Window();
        try
        {
            var heard = new List<string>();
            using (var hook = new WindowHook((nint)window.Value))
            {
                hook.Locked += locked => heard.Add(locked ? "locked" : "unlocked");
                hook.Connected += connected => heard.Add(connected ? "connected" : "disconnected");
                // WTS_CONSOLE_DISCONNECT, _CONSOLE_CONNECT, _REMOTE_DISCONNECT, _REMOTE_CONNECT,
                // _SESSION_LOCK, _SESSION_UNLOCK, _SESSION_LOGON.
                foreach (var reason in new nuint[] { 2, 1, 4, 3, 7, 8, 5 })
                {
                    SendMessageW(window, WmWtsSessionChange, reason, 0);
                }
            }
            Assert.Equal(["disconnected", "connected", "disconnected", "connected", "locked", "unlocked"], heard);
        }
        finally
        {
            DestroyWindow(window);
        }
    }

    [Fact]
    public void AHandlerThatFailsIsLoggedByItsTypeOnly()
    {
        var lines = new List<string>();
        var write = InkLog.Write;
        InkLog.Write = lines.Add;
        var window = Window();
        try
        {
            using (var hook = new WindowHook((nint)window.Value))
            {
                hook.TaskbarButtonCreated += () => throw new InvalidOperationException("a private detail");
                SendMessageW(window, TaskbarButtonCreated(), 0, 0);
            }
            Assert.Contains(lines, line => line.Contains(nameof(InvalidOperationException), StringComparison.Ordinal));
            Assert.DoesNotContain(lines, line => line.Contains("a private detail", StringComparison.Ordinal));
        }
        finally
        {
            InkLog.Write = write;
            DestroyWindow(window);
        }
    }
}
