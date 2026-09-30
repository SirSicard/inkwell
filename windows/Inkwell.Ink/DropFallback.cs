// The Drop when the ink cannot draw: the Drop is the recording indicator, so it never goes blank.
// While Direct3D is starting, lost (TDR, driver update, DWM restart) or unable to compile the
// shader, this plain window takes its place: the paper panel, a still ink dot and the state's two
// lines, painted with GDI into an ordinary redirected window, so it depends on neither Direct3D nor
// DirectComposition. It keeps the Drop's rules: WS_EX_NOACTIVATE | WS_EX_TOPMOST |
// WS_EX_TOOLWINDOW, MA_NOACTIVATE, shown only with SWP_NOACTIVATE.
using System.Runtime.InteropServices;
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

/// <summary>The Drop's plain stand-in. UI thread only.</summary>
internal sealed unsafe class DropFallback : IDisposable
{
    private const string ClassName = "InkwellDropFallback";
    // wingdi.h (Windows SDK 10.0.26100.0).
    private const int NullBrush = 5, NullPen = 8;
    private const uint DefaultCharset = 1, OutDefaultPrecis = 0, DefaultPitch = 0;
    private static bool registered;

    private GCHandle self;
    private HWND hwnd;
    private DropText text = new("", "");
    private double scale = 1;
    /// <summary>The Drop's screen reader name, over it (the Drop's window announces it).</summary>
    private readonly ScreenReaderName speech;

    /// <summary>Whether it is on screen.</summary>
    public bool IsShown { get; private set; }

    /// <summary>The window's handle (tests).</summary>
    public nint Handle => (nint)hwnd.Value;

    public DropFallback()
    {
        self = GCHandle.Alloc(this);
        var instance = GetModuleHandleW(null);
        fixed (char* name = ClassName)
        {
            if (!registered)
            {
                var wc = new WNDCLASSEXW
                {
                    cbSize = (uint)sizeof(WNDCLASSEXW),
                    lpfnWndProc = &WndProc,
                    hInstance = (HINSTANCE)instance,
                    lpszClassName = name,
                    hCursor = LoadCursorW(HINSTANCE.NULL, IDC.IDC_ARROW),
                };
                if (RegisterClassExW(&wc) == 0)
                {
                    var error = GetLastError();
                    self.Free();
                    throw new InkRendererException($"couldn't register the Drop's fallback window class (error {error})");
                }
                registered = true;
            }
            hwnd = CreateWindowExW(
                (uint)(WS.WS_EX_NOACTIVATE | WS.WS_EX_TOPMOST | WS.WS_EX_TOOLWINDOW),
                name, name, unchecked((uint)WS.WS_POPUP), 0, 0, 1, 1, HWND.NULL, HMENU.NULL, (HINSTANCE)instance, null);
        }
        if (hwnd == HWND.NULL)
        {
            var error = GetLastError();
            self.Free();
            throw new InkRendererException($"couldn't make the Drop's fallback window (error {error})");
        }
        SetWindowLongPtrW(hwnd, GWLP.GWLP_USERDATA, GCHandle.ToIntPtr(self));
        speech = new ScreenReaderName(hwnd, "the Drop's fallback");
    }

    /// <summary>Shows <paramref name="lines"/> over <paramref name="bounds"/> (the Drop's rectangle, in pixels) at <paramref name="dpiScale"/>.</summary>
    public void Show(RECT bounds, DropText lines, double dpiScale)
    {
        text = lines;
        scale = dpiScale;
        int w = bounds.right - bounds.left, h = bounds.bottom - bounds.top;
        var corner = (int)Math.Round(2 * DropLayout.CornerRadius * scale);
        // The window owns the region once set. Without it the panel is square: still shown.
        _ = SetWindowRgn(hwnd, CreateRoundRectRgn(0, 0, w + 1, h + 1, corner, corner), false);
        fixed (char* name = lines.WindowTitle)
        {
            SetWindowTextW(hwnd, name);
        }
        speech.Set(lines.AccessibleName, announce: false);
        SetWindowPos(hwnd, HWND.HWND_TOPMOST, bounds.left, bounds.top, w, h,
            SWP.SWP_NOACTIVATE | SWP.SWP_SHOWWINDOW);
        InvalidateRect(hwnd, null, true);
        IsShown = true;
    }

    public void Hide()
    {
        if (IsShown)
        {
            ShowWindow(hwnd, SW.SW_HIDE);
            IsShown = false;
            speech.Clear();
        }
    }

    private static COLORREF Colour((float R, float G, float B) c) =>
        (COLORREF)((uint)Math.Round(c.R * 255) | ((uint)Math.Round(c.G * 255) << 8) | ((uint)Math.Round(c.B * 255) << 16));

    // A GDI call that fails here leaves part of the panel unpainted until the next WM_PAINT; the
    // panel still shows. So their results are discarded on purpose.
    private void Paint()
    {
        PAINTSTRUCT ps;
        var dc = BeginPaint(hwnd, &ps);
        RECT client;
        GetClientRect(hwnd, &client);
        int S(double dips) => (int)Math.Round(dips * scale);

        var paper = CreateSolidBrush(Colour(Palette.Paper));
        _ = FillRect(dc, &client, paper);
        DeleteObject((HGDIOBJ)paper.Value);

        // A still drop of ink where the ink would be.
        var ink = CreateSolidBrush(Colour(Palette.Ink));
        var noPen = GetStockObject(NullPen);
        var oldPen = SelectObject(dc, noPen);
        var oldBrush = SelectObject(dc, (HGDIOBJ)ink.Value);
        int cx = S(DropLayout.InkWidth / 2), cy = (client.bottom - client.top) / 2, r = S(22);
        Ellipse(dc, cx - r, cy - r, cx + r, cy + r);
        SelectObject(dc, oldBrush);
        DeleteObject((HGDIOBJ)ink.Value);

        // The border: seal red for an alert, else a light hairline.
        var alert = text.Tone == DropTone.Alert;
        var border = CreatePen(PS.PS_SOLID, Math.Max(1, S(alert ? 1.5 : 1)), Colour(alert ? Palette.Seal : (0.85f, 0.84f, 0.81f)));
        SelectObject(dc, (HGDIOBJ)border.Value);
        var hollow = SelectObject(dc, GetStockObject(NullBrush));
        var corner = S(2 * DropLayout.CornerRadius);
        RoundRect(dc, 0, 0, client.right, client.bottom, corner, corner);
        SelectObject(dc, hollow);
        SelectObject(dc, oldPen);
        DeleteObject((HGDIOBJ)border.Value);

        // The two lines.
        _ = SetBkMode(dc, TRANSPARENT);
        var titleFont = Font(S(DropLayout.TitleSize), FW.FW_MEDIUM);
        var detailFont = Font(S(DropLayout.DetailSize), FW.FW_NORMAL);
        var left = S(DropLayout.TextLeft);
        var right = client.right - S(DropLayout.TextRight);
        var mid = (client.bottom - client.top) / 2;
        var titleRect = new RECT { left = left, top = mid - S(20), right = right, bottom = mid - S(1) };
        var detailRect = new RECT { left = left, top = mid + S(1), right = right, bottom = mid + S(22) };
        var oldFont = SelectObject(dc, (HGDIOBJ)titleFont.Value);
        SetTextColor(dc, Colour(text.Tone == DropTone.Plain ? Palette.Muted : Palette.Seal));
        fixed (char* t = text.Title)
        {
            _ = DrawTextW(dc, t, text.Title.Length, &titleRect, DT.DT_LEFT | DT.DT_BOTTOM | DT.DT_SINGLELINE | DT.DT_END_ELLIPSIS | DT.DT_NOPREFIX);
        }
        SelectObject(dc, (HGDIOBJ)detailFont.Value);
        SetTextColor(dc, Colour(Palette.Ink));
        fixed (char* d = text.Detail)
        {
            _ = DrawTextW(dc, d, text.Detail.Length, &detailRect, DT.DT_LEFT | DT.DT_TOP | DT.DT_SINGLELINE | DT.DT_END_ELLIPSIS | DT.DT_NOPREFIX);
        }
        SelectObject(dc, oldFont);
        DeleteObject((HGDIOBJ)titleFont.Value);
        DeleteObject((HGDIOBJ)detailFont.Value);
        EndPaint(hwnd, &ps);
    }

    private static HFONT Font(int pixels, int weight)
    {
        fixed (char* face = "Segoe UI")
        {
            return CreateFontW(-pixels, 0, 0, 0, weight, 0, 0, 0, DefaultCharset, OutDefaultPrecis, CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY, DefaultPitch, face);
        }
    }

    [UnmanagedCallersOnly]
    private static LRESULT WndProc(HWND hwnd, uint msg, WPARAM wParam, LPARAM lParam)
    {
        // Nothing may escape an UnmanagedCallersOnly method: that ends the process.
        try
        {
            return Dispatch(hwnd, msg, wParam, lParam);
        }
        catch (Exception e)
        {
            InkLog.Write($"the Drop's fallback window procedure failed (message 0x{msg:X4}): {e.GetType().Name}: {e.Message}");
            return DefWindowProcW(hwnd, msg, wParam, lParam);
        }
    }

    private static LRESULT Dispatch(HWND hwnd, uint msg, WPARAM wParam, LPARAM lParam)
    {
        switch (msg)
        {
            case WM.WM_MOUSEACTIVATE:
                return MA.MA_NOACTIVATE;
            case WM.WM_PAINT:
                var handle = GetWindowLongPtrW(hwnd, GWLP.GWLP_USERDATA);
                if (handle != 0 && GCHandle.FromIntPtr(handle).Target is DropFallback fallback)
                {
                    fallback.Paint();
                    return 0;
                }
                break;
            default:
                break;
        }
        return DefWindowProcW(hwnd, msg, wParam, lParam);
    }

    public void Dispose()
    {
        speech.Dispose();
        if (hwnd != HWND.NULL)
        {
            SetWindowLongPtrW(hwnd, GWLP.GWLP_USERDATA, 0);
            DestroyWindow(hwnd);
            hwnd = HWND.NULL;
        }
        if (self.IsAllocated)
        {
            self.Free();
        }
    }
}
