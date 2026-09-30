// What a screen reader gets from a window, read as Narrator reads it: a UI Automation client (the
// element's name and live setting) and a WinEvent hook counting live region announcements. The
// UI Automation declarations are UIAutomationClient.h's (Windows SDK 10.0.26100.0), which TerraFX
// does not carry.
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using TerraFX.Interop.Windows;
using Xunit;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink.Tests;

internal static unsafe class Uia
{
    // UIAutomationClient.h: CLSID_CUIAutomation, IID_IUIAutomation, the property ids.
    private static readonly Guid Clsid = new("ff48dba4-60ef-4201-aa87-54103eef594e");
    private static readonly Guid Iid = new("30cbe57d-d9d0-452a-ab13-7ac5ac4825ee");
    private const int NamePropertyId = 30005, LiveSettingPropertyId = 30135;
    // IUIAutomation after IUnknown: CompareElements, CompareRuntimeIds, GetRootElement,
    // ElementFromHandle. IUIAutomationElement after IUnknown: SetFocus, GetRuntimeId, FindFirst,
    // FindAll, FindFirstBuildCache, FindAllBuildCache, BuildUpdatedCache, GetCurrentPropertyValue.
    private const int ElementFromHandleSlot = 6, GetCurrentPropertyValueSlot = 10;

    /// <summary>
    /// The UI Automation name and live setting of <paramref name="hwnd"/>'s element. UI Automation
    /// asks the window's thread, so the read runs on its own (MTA) thread while
    /// <paramref name="ui"/>, the window's thread, pumps.
    /// </summary>
    public static (string? Name, int LiveSetting) Read(nint hwnd, UiThread ui)
    {
        (string?, int) result = default;
        Exception? failure = null;
        var thread = new Thread(() =>
        {
            try
            {
                result = ReadNow(hwnd);
            }
            catch (Exception e)
            {
                failure = e;
            }
        });
        thread.SetApartmentState(ApartmentState.MTA);
        thread.Start();
        var until = DateTime.UtcNow.AddSeconds(10);
        while (thread.IsAlive && DateTime.UtcNow < until)
        {
            ui.Pump(0.01);
        }
        Assert.False(thread.IsAlive, "UI Automation answered within 10 s");
        if (failure is not null)
        {
            throw new InvalidOperationException("UI Automation could not read the window", failure);
        }
        return result;
    }

    private static (string?, int) ReadNow(nint hwnd)
    {
        IUnknown* automation;
        fixed (Guid* clsid = &Clsid)
        fixed (Guid* iid = &Iid)
        {
            Check(CoCreateInstance(clsid, null, (uint)CLSCTX.CLSCTX_INPROC_SERVER, iid, (void**)&automation), "make the UI Automation client");
        }
        try
        {
            IUnknown* element;
            var fromHandle = (delegate* unmanaged[MemberFunction]<IUnknown*, void*, IUnknown**, int>)(*(void***)automation)[ElementFromHandleSlot];
            Check(fromHandle(automation, (void*)hwnd, &element), "find the window's element");
            try
            {
                var get = (delegate* unmanaged[MemberFunction]<IUnknown*, int, VARIANT*, int>)(*(void***)element)[GetCurrentPropertyValueSlot];
                VARIANT v;
                Check(get(element, NamePropertyId, &v), "read the name");
                var name = v.vt == (ushort)VARENUM.VT_BSTR && v.bstrVal != null ? new string(v.bstrVal) : null;
                _ = VariantClear(&v);
                Check(get(element, LiveSettingPropertyId, &v), "read the live setting");
                var live = v.vt == (ushort)VARENUM.VT_I4 ? v.lVal : -1;
                _ = VariantClear(&v);
                return (name, live);
            }
            finally
            {
                element->Release();
            }
        }
        finally
        {
            automation->Release();
        }
    }

    private static void Check(int hr, string what)
    {
        if (hr < 0)
        {
            throw new InvalidOperationException($"couldn't {what} (0x{hr:X8})");
        }
    }
}

/// <summary>
/// Counts EVENT_OBJECT_LIVEREGIONCHANGED (UI Automation's LiveRegionChanged) per window, raised in
/// this process on the whole element (OBJID_CLIENT, CHILDID_SELF). Delivered while the hooking
/// thread pumps messages.
/// </summary>
internal sealed unsafe class LiveRegionEvents : IDisposable
{
    /// <summary>The instance listening (the hook's callback is static).</summary>
    private static LiveRegionEvents? listening;
    private readonly ConcurrentDictionary<nint, int> counts = new();
    private readonly HWINEVENTHOOK hook;

    public LiveRegionEvents()
    {
        listening = this;
        hook = SetWinEventHook(EVENT.EVENT_OBJECT_LIVEREGIONCHANGED, EVENT.EVENT_OBJECT_LIVEREGIONCHANGED, HMODULE.NULL, &Heard,
            (uint)Environment.ProcessId, 0, WINEVENT_OUTOFCONTEXT);
        Assert.True(hook != HWINEVENTHOOK.NULL, $"couldn't hook live region events (error {GetLastError()})");
    }

    /// <summary>How many announcements <paramref name="hwnd"/> made.</summary>
    public int For(nint hwnd) => counts.GetValueOrDefault(hwnd);

    [UnmanagedCallersOnly]
    private static void Heard(HWINEVENTHOOK hook, uint e, HWND hwnd, int idObject, int idChild, uint thread, uint time)
    {
        if (idObject == OBJID.OBJID_CLIENT && idChild == CHILDID_SELF)
        {
            listening?.counts.AddOrUpdate((nint)hwnd.Value, 1, (_, n) => n + 1);
        }
    }

    public void Dispose()
    {
        UnhookWinEvent(hook);
        listening = null;
    }
}
