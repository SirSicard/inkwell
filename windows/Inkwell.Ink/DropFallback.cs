// The Drop when the ink cannot draw: the Drop is the recording indicator, so it never goes blank.
// While Direct3D is starting, lost (TDR, driver update, DWM restart) or unable to compile the
// shader, this plain window takes its place: the paper panel, a still ink dot and the state's two
// lines, painted with GDI into an ordinary redirected window, so it depends on neither Direct3D nor
// DirectComposition. It keeps the Drop's rules: WS_EX_NOACTIVATE | WS_EX_TOPMOST |
// WS_EX_TOOLWINDOW, MA_NOACTIVATE, shown only with SWP_NOACTIVATE. An offer's buttons are drawn
// and answer clicks here too, where the Drop's own layout puts them (DropLayout).
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
    /// <summary>The button a press went down on, until it comes up.</summary>
    private int? pressed;

    /// <summary>A button of the text shown was clicked (its index). UI thread.</summary>
    public event Action<int>? ButtonClicked;

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
        if (lines != text)
        {
            pressed = null;
        }
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
        var (titleRect, detailRect, detailFormat) = Lines(text.Buttons is not null, client, scale);
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
            _ = DrawTextW(dc, d, text.Detail.Length, &detailRect, detailFormat);
        }
        if (text.Buttons is { } buttons)
        {
            // The first in ink with paper words (the answer), the second outlined in ink.
            var buttonFont = Font(S(DropLayout.ButtonTextSize), FW.FW_MEDIUM);
            SelectObject(dc, (HGDIOBJ)buttonFont.Value);
            var inkPen = CreatePen(PS.PS_SOLID, Math.Max(1, S(1)), Colour(Palette.Ink));
            var inkFill = CreateSolidBrush(Colour(Palette.Ink));
            var penBefore = SelectObject(dc, (HGDIOBJ)inkPen.Value);
            for (var i = 0; i < buttons.Count; i++)
            {
                var (bl, bt, br, bb) = DropLayout.Button(i);
                var rect = new RECT { left = S(bl), top = S(bt), right = S(br), bottom = S(bb) };
                var fillBefore = SelectObject(dc, i == 0 ? (HGDIOBJ)inkFill.Value : GetStockObject(NullBrush));
                RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, S(12), S(12));
                SelectObject(dc, fillBefore);
                SetTextColor(dc, Colour(i == 0 ? Palette.Paper : Palette.Ink));
                var words = buttons[i];
                fixed (char* w = words)
                {
                    _ = DrawTextW(dc, w, words.Length, &rect, DT.DT_CENTER | DT.DT_VCENTER | DT.DT_SINGLELINE | DT.DT_END_ELLIPSIS | DT.DT_NOPREFIX);
                }
            }
            SelectObject(dc, penBefore);
            DeleteObject((HGDIOBJ)inkPen.Value);
            DeleteObject((HGDIOBJ)inkFill.Value);
            SelectObject(dc, oldFont);
            DeleteObject((HGDIOBJ)buttonFont.Value);
        }
        SelectObject(dc, oldFont);
        DeleteObject((HGDIOBJ)titleFont.Value);
        DeleteObject((HGDIOBJ)detailFont.Value);
        EndPaint(hwnd, &ps);
    }

    /// <summary>
    /// Where the two lines go in a panel whose client area is <paramref name="client"/>, and how the
    /// detail is drawn. Centred on the panel, one line each. With buttons (the consent offer), over
    /// them, the detail on up to two lines, as the ink's Drop draws it: the offer's consent
    /// sentence is the one line the Drop must not cut.
    /// </summary>
    internal static (RECT Title, RECT Detail, uint DetailFormat) Lines(bool buttons, RECT client, double scale)
    {
        int S(double dips) => (int)Math.Round(dips * scale);
        var left = S(DropLayout.TextLeft);
        var right = client.right - S(DropLayout.TextRight);
        var mid = buttons ? S(25) : (client.bottom - client.top) / 2;
        var title = new RECT { left = left, top = mid - S(20), right = right, bottom = mid - S(1) };
        var detail = new RECT { left = left, top = mid + S(1), right = right, bottom = mid + S(buttons ? 41 : 22) };
        var format = (uint)(DT.DT_LEFT | DT.DT_TOP | DT.DT_END_ELLIPSIS | DT.DT_NOPREFIX
            | (buttons ? DT.DT_WORDBREAK | DT.DT_EDITCONTROL : DT.DT_SINGLELINE));
        return (title, detail, format);
    }

    /// <summary>The fallback's face at <paramref name="pixels"/> high.</summary>
    internal static HFONT Font(int pixels, int weight)
    {
        fixed (char* face = "Segoe UI")
        {
            return CreateFontW(-pixels, 0, 0, 0, weight, 0, 0, 0, DefaultCharset, OutDefaultPrecis, CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY, DefaultPitch, face);
        }
    }

    /// <summary>A press at <paramref name="lParam"/> (client pixels): a click is down and up on the same button.</summary>
    private void Mouse(bool down, LPARAM lParam)
    {
        var x = (short)((nint)lParam & 0xFFFF) / scale;
        var y = (short)(((nint)lParam >> 16) & 0xFFFF) / scale;
        var button = DropLayout.ButtonAt(text.Buttons, x, y);
        if (down)
        {
            pressed = button;
            return;
        }
        var was = pressed;
        pressed = null;
        if (button is int index && index == was)
        {
            ButtonClicked?.Invoke(index);
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
                if (From(hwnd) is { } fallback)
                {
                    fallback.Paint();
                    return 0;
                }
                break;
            case WM.WM_LBUTTONDOWN:
            case WM.WM_LBUTTONUP:
                if (From(hwnd) is { IsShown: true } clicked)
                {
                    clicked.Mouse(msg == WM.WM_LBUTTONDOWN, lParam);
                    return 0;
                }
                break;
            default:
                break;
        }
        return DefWindowProcW(hwnd, msg, wParam, lParam);
    }

    private static DropFallback? From(HWND hwnd)
    {
        var handle = GetWindowLongPtrW(hwnd, GWLP.GWLP_USERDATA);
        return handle == 0 ? null : GCHandle.FromIntPtr(handle).Target as DropFallback;
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
