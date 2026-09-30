// What a screen reader reads for one of the Drop's windows, apart from its title: the name the
// Mac's Drop gives VoiceOver (its accessibility label, mac/Sources/Inkwell/Drop.swift), live words
// included, and a polite live setting. Both are set through Direct Annotation (oleacc's
// IAccPropServices), which UI Automation reads for the window, so the window's title, which any
// process reads with GetWindowText, stays the state only. A new name is announced with
// EVENT_OBJECT_LIVEREGIONCHANGED, which UI Automation passes on as its LiveRegionChanged event:
// Narrator reads it once it has finished what it is saying, and the window never takes focus.
//
// The declarations are the Windows SDK's (10.0.26100.0: oleacc.h, UIAutomationCoreApi.h and
// UIAutomationCore.h), which TerraFX does not carry. The words are the user's: nothing here logs
// them.
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

/// <summary>A window's screen reader name and live setting. UI thread only (the window's).</summary>
internal sealed unsafe class ScreenReaderName : IDisposable
{
    // oleacc.h: CLSID_AccPropServices, IID_IAccPropServices, PROPID_ACC_NAME.
    private static readonly Guid ClsidAccPropServices = new("b5f8350b-0548-48b1-a6ee-88bd00b4a5e7");
    private static readonly Guid IidAccPropServices = new("6e26e776-04f0-495d-80e4-3330352e3169");
    private static readonly Guid NameProperty = new(0x608d3df8, 0x8128, 0x4aa7, 0xa4, 0x28, 0xf5, 0x5e, 0x49, 0x26, 0x72, 0x91);
    // UIAutomationCoreApi.h: LiveSetting_Property_GUID.
    private static readonly Guid LiveSettingProperty = new(0xc12bcd8e, 0x2a8e, 0x4950, 0x8a, 0xe7, 0x36, 0x25, 0x11, 0x1d, 0x58, 0xeb);
    /// <summary>UIAutomationCore.h: LiveSetting's Polite.</summary>
    internal const int Polite = 1;
    // IAccPropServices' methods after IUnknown's three (oleacc.h): SetPropValue, SetPropServer,
    // ClearProps, SetHwndProp, SetHwndPropStr, SetHwndPropServer, ClearHwndProps.
    private const int SetHwndPropSlot = 6, SetHwndPropStrSlot = 7, ClearHwndPropsSlot = 9;
    private const uint Client = unchecked((uint)OBJID.OBJID_CLIENT), Self = CHILDID_SELF;

    private readonly HWND hwnd;
    /// <summary>The window, as the log names it ("the Drop").</summary>
    private readonly string window;
    private IUnknown* services;
    private string? lastFailure;

    /// <summary>The name screen readers are given now (null: none, they read the window's title).</summary>
    public string? Name { get; private set; }

    /// <summary>
    /// For <paramref name="hwnd"/>, which <paramref name="window"/> names in the log. Without
    /// Windows' annotation service screen readers read the window's title only, and the log says
    /// why.
    /// </summary>
    public ScreenReaderName(HWND hwnd, string window)
    {
        this.hwnd = hwnd;
        this.window = window;
        IUnknown* s;
        HRESULT hr;
        fixed (Guid* clsid = &ClsidAccPropServices)
        fixed (Guid* iid = &IidAccPropServices)
        {
            hr = CoCreateInstance(clsid, null, (uint)CLSCTX.CLSCTX_INPROC_SERVER, iid, (void**)&s);
        }
        if (Failed(hr, $"reach Windows' accessibility annotations; screen readers read {window}'s title only"))
        {
            return;
        }
        services = s;
        var live = default(VARIANT);
        live.vt = (ushort)VARENUM.VT_I4;
        live.lVal = Polite;
        var setProp = (delegate* unmanaged[MemberFunction]<IUnknown*, HWND, uint, uint, Guid, VARIANT, int>)(*(void***)services)[SetHwndPropSlot];
        _ = Failed(setProp(services, hwnd, Client, Self, LiveSettingProperty, live),
            $"make {window} a live region; screen readers may not read its changes");
    }

    /// <summary>
    /// Gives screen readers <paramref name="name"/>. When it is new and <paramref name="announce"/>,
    /// a screen reader reads it out (politely, without focus).
    /// </summary>
    public void Set(string name, bool announce)
    {
        ArgumentNullException.ThrowIfNull(name);
        if (name == Name)
        {
            return;
        }
        Name = name;
        if (services == null)
        {
            return;
        }
        var setString = (delegate* unmanaged[MemberFunction]<IUnknown*, HWND, uint, uint, Guid, char*, int>)(*(void***)services)[SetHwndPropStrSlot];
        HRESULT hr;
        fixed (char* s = name)
        {
            hr = setString(services, hwnd, Client, Self, NameProperty, s);
        }
        if (!Failed(hr, $"give {window} its screen reader name; screen readers read its title only") && announce)
        {
            NotifyWinEvent(EVENT.EVENT_OBJECT_LIVEREGIONCHANGED, hwnd, OBJID.OBJID_CLIENT, CHILDID_SELF);
        }
    }

    /// <summary>Takes the name away: screen readers read the window's title (the state only), never the last words.</summary>
    public void Clear()
    {
        if (Name is null)
        {
            return;
        }
        Name = null;
        ClearProperties([NameProperty], $"take {window}'s screen reader name away");
    }

    private void ClearProperties(ReadOnlySpan<Guid> properties, string what)
    {
        if (services == null)
        {
            return;
        }
        var clear = (delegate* unmanaged[MemberFunction]<IUnknown*, HWND, uint, uint, Guid*, int, int>)(*(void***)services)[ClearHwndPropsSlot];
        HRESULT hr;
        fixed (Guid* p = properties)
        {
            hr = clear(services, hwnd, Client, Self, p, properties.Length);
        }
        _ = Failed(hr, what);
    }

    /// <summary>Whether <paramref name="hr"/> failed; a failure is logged ("couldn't " and <paramref name="what"/>) once until a call succeeds.</summary>
    private bool Failed(HRESULT hr, string what)
    {
        if (hr.SUCCEEDED)
        {
            lastFailure = null;
            return false;
        }
        var message = $"couldn't {what} (0x{hr.Value:X8})";
        if (message != lastFailure)
        {
            lastFailure = message;
            InkLog.Write(message);
        }
        return true;
    }

    /// <summary>Takes both annotations away (before the window is destroyed) and releases the service.</summary>
    public void Dispose()
    {
        Name = null;
        ClearProperties([NameProperty, LiveSettingProperty], $"take {window}'s screen reader annotations away");
        Com.Release(ref services);
    }
}
