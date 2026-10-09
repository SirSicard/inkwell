// The main window's taskbar button (ITaskbarList3): an overlay badge with its spoken description,
// the progress bar (the final pass's, and the error state for a problem), and the thumbnail
// toolbar's one button (Record or Stop). Made once Windows says the button exists
// (WindowHook.TaskbarButtonCreated); until then, and if the shell refuses, every call does nothing
// and returns false, and each kind of failure is logged once per connection: the tray icon and the
// Drop still show the state. UI thread.
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

/// <summary>The progress bar's states (TBPFLAG).</summary>
public enum TaskbarProgress
{
    None = 0,
    Indeterminate = 0x1,
    Normal = 0x2,
    Error = 0x4,
}

public sealed unsafe class TaskbarButton : IDisposable
{
    /// <summary>The thumbnail toolbar's button id.</summary>
    public const int RecordButtonId = 1;

    private readonly HWND window;
    private ITaskbarList3* list;
    private bool buttonsAdded;
    /// <summary>The kinds of failure logged since the last connection: each once.</summary>
    private readonly HashSet<string> logged = [];

    public TaskbarButton(nint window) => this.window = (HWND)window;

    /// <summary>The button exists now (or exists again after Explorer restarted): the shell's object is made afresh.</summary>
    public bool Connect()
    {
        Release();
        buttonsAdded = false;
        ITaskbarList3* made;
        var clsid = CLSID.CLSID_TaskbarList;
        var iid = IID.IID_ITaskbarList3;
        var hr = CoCreateInstance(&clsid, null, (uint)CLSCTX.CLSCTX_INPROC_SERVER, &iid, (void**)&made);
        if (hr < 0)
        {
            Fail($"couldn't reach the taskbar button (0x{(int)hr:X8})");
            return false;
        }
        hr = made->HrInit();
        if (hr < 0)
        {
            _ = made->Release();
            Fail($"couldn't start the taskbar button (0x{(int)hr:X8})");
            return false;
        }
        list = made;
        logged.Clear();
        return true;
    }

    /// <summary>The overlay badge (0: none) and what Narrator says for it; whether the shell took it.</summary>
    public bool Overlay(nint icon, string? description)
    {
        if (list is null)
        {
            return false;
        }
        fixed (char* text = description ?? "")
        {
            return Check(list->SetOverlayIcon(window, (HICON)icon, description is null ? null : text), "set the taskbar badge");
        }
    }

    /// <summary>The progress bar: its state, and how far, 0..1 (ignored for None and Indeterminate).</summary>
    public bool Progress(TaskbarProgress state, double done)
    {
        if (list is null)
        {
            return false;
        }
        var ok = Check(list->SetProgressState(window, (TBPFLAG)state), "set the taskbar progress");
        if (ok && state is TaskbarProgress.Normal or TaskbarProgress.Error)
        {
            ok = Check(list->SetProgressValue(window, (ulong)Math.Round(Math.Clamp(done, 0, 1) * 1000), 1000), "set the taskbar progress");
        }
        return ok;
    }

    /// <summary>The thumbnail toolbar's button: its picture, tooltip (Narrator reads it) and whether it can be pressed.</summary>
    public bool Button(nint icon, string tip, bool enabled)
    {
        if (list is null)
        {
            return false;
        }
        var button = new THUMBBUTTON
        {
            dwMask = THUMBBUTTONMASK.THB_ICON | THUMBBUTTONMASK.THB_TOOLTIP | THUMBBUTTONMASK.THB_FLAGS,
            iId = RecordButtonId,
            hIcon = (HICON)icon,
            dwFlags = enabled ? THUMBBUTTONFLAGS.THBF_ENABLED : THUMBBUTTONFLAGS.THBF_DISABLED,
        };
        var room = System.Runtime.InteropServices.MemoryMarshal.CreateSpan(ref button.szTip[0], 260);
        room.Clear();
        tip.AsSpan(0, Math.Min(tip.Length, 259)).CopyTo(room);
        // Added once: the toolbar's buttons cannot be removed, only updated.
        if (!buttonsAdded)
        {
            buttonsAdded = Check(list->ThumbBarAddButtons(window, 1, &button), "add the thumbnail button");
            return buttonsAdded;
        }
        return Check(list->ThumbBarUpdateButtons(window, 1, &button), "update the thumbnail button");
    }

    private bool Check(HRESULT hr, string what)
    {
        if (hr >= 0)
        {
            return true;
        }
        Fail($"couldn't {what} (0x{(int)hr:X8})");
        return false;
    }

    /// <summary>Said once for each kind until the next connection: the state still shows in the tray and the Drop.</summary>
    private void Fail(string message)
    {
        if (logged.Add(message))
        {
            InkLog.Write(message);
        }
    }

    private void Release()
    {
        if (list is not null)
        {
            _ = list->Release();
            list = null;
        }
    }

    public void Dispose() => Release();
}
