// The Drop: the surface for everything live, as on the Mac (mac/Sources/Inkwell/Drop.swift). A
// pill near the bottom of the screen in the app's mode (night or day: DropLook), the orb in a
// circle on its left and two lines beside it, shown while something is live and hidden when idle.
//
// It never takes focus. It is a raw Win32 popup: WS_EX_NOACTIVATE (clicks and showing never
// activate it), WS_EX_TOPMOST (above other apps' windows), WS_EX_TOOLWINDOW (no taskbar button,
// not in Alt-Tab), and it answers WM_MOUSEACTIVATE with MA_NOACTIVATE. It is shown with
// SetWindowPos(SWP_NOACTIVATE), never ShowWindow(SW_SHOW). windows/S3.4-CHECKLIST.md is the check
// by hand.
//
// Screen readers read its title and detail, live words included, as VoiceOver reads the Mac's, in
// a polite live region (ScreenReaderName); the window's title, which any process reads, is the
// state only.
//
// Its pixels: WS_EX_NOREDIRECTIONBITMAP (no GDI surface at all), a DirectComposition visual
// holding a composition swapchain. Each frame Direct3D draws the orb into an offscreen texture
// (transparent where there is no orb), then Direct2D paints the pill on the swapchain: the mode's
// background with fully round ends, the orb clipped to its circle, a hairline border, the state in
// Segoe UI Variable and the line in Sitka. Frames follow the ink's schedule (InkSurface): live, on
// the shared clock; with Animation effects off or Always still, one still frame per change;
// hidden, none.
//
// The consent offer's buttons ("Record this call", "Not this one") widen and deepen the pill, as
// on the Mac. A click on one (down and up on the same button) raises ButtonClicked; the click
// still never activates the Drop or Inkwell (MA_NOACTIVATE).
//
// The Drop is the recording indicator, so it never goes blank. A lost device (TDR, driver update),
// a lost composition device (DWM restarted) or a failed present releases every Direct3D,
// Direct2D and DirectComposition object here; the surface makes the pipeline again and retries
// with backoff (InkSurface). Until it draws again, or when the shader does not compile at all,
// DropFallback, a plain GDI window in the same mode, shows the pill and the state's lines in its place.
using System.Runtime.InteropServices;
using TerraFX.Interop.DirectX;
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

/// <summary>The Drop's measures, in DIPs (the canvas's pill).</summary>
internal static class DropLayout
{
    public const double Width = 440;
    public const double Height = 76;
    /// <summary>The orb's circle, and its inset from the pill's left and top edges.</summary>
    public const double OrbSize = 58;
    public const double OrbInset = (Height - OrbSize) / 2;
    /// <summary>The orb's zone, left of the lines: as wide as the pill is high.</summary>
    public const double InkWidth = Height;
    /// <summary>Fully round ends.</summary>
    public const double CornerRadius = Height / 2;
    /// <summary>Above the bottom of the work area (the taskbar's top when it shows).</summary>
    public const double BottomMargin = 28;
    public const double TextLeft = OrbInset + OrbSize + 14;
    public const double TextRight = 24;
    public const double LineSpacing = 2;
    public const float TitleSize = 12;
    public const float DetailSize = 17;
    public const string Face = "Segoe UI Variable Text";
    /// <summary>The line's face: the display serif.</summary>
    public const string DetailFace = "Sitka Text";

    /// <summary>With buttons (the consent offer): wider and taller, as the Mac's sizeWithActions.</summary>
    public const double WidthWithButtons = 480;
    public const double HeightWithButtons = 132;
    public const double ButtonWidth = 150;
    public const double ButtonHeight = 30;
    public const double ButtonGap = 8;
    /// <summary>From the panel's bottom edge to the buttons'.</summary>
    public const double ButtonBottom = 14;
    public const float ButtonTextSize = 12.5f;

    /// <summary>The panel's size for <paramref name="text"/>, in DIPs.</summary>
    public static (double W, double H) Size(DropText text) =>
        text.Buttons is null ? (Width, Height) : (WidthWithButtons, HeightWithButtons);

    /// <summary>Button <paramref name="index"/>'s rectangle, in DIPs from the panel's top left: a row under the lines.</summary>
    public static (double Left, double Top, double Right, double Bottom) Button(int index)
    {
        var left = TextLeft + index * (ButtonWidth + ButtonGap);
        var top = HeightWithButtons - ButtonBottom - ButtonHeight;
        return (left, top, left + ButtonWidth, top + ButtonHeight);
    }

    /// <summary>The index of the button of <paramref name="buttons"/> at (<paramref name="x"/>, <paramref name="y"/>) in DIPs, or null.</summary>
    public static int? ButtonAt(DropButtons? buttons, double x, double y)
    {
        for (var i = 0; i < (buttons?.Count ?? 0); i++)
        {
            var (left, top, right, bottom) = Button(i);
            if (x >= left && x < right && y >= top && y < bottom)
            {
                return i;
            }
        }
        return null;
    }
}

/// <summary>The Drop's window. UI thread only (the thread whose message loop it lives on).</summary>
public sealed unsafe class DropWindow : IInkTarget, IDisposable
{
    private const string ClassName = "InkwellDrop";
    private static bool registered;

    private GCHandle self;
    private HWND hwnd;
    private IDCompositionDevice* composition;
    private IDCompositionTarget* compositionTarget;
    private IDCompositionVisual* visual;
    private CompositionSwapChain? swapChain;
    private InkTexture? inkTexture;
    private ID2D1Bitmap1* inkBitmap;
    private IDWriteTextFormat* titleFormat;
    private IDWriteTextFormat* detailFormat;
    private IDWriteTextFormat* buttonFormat;
    private IDWriteTextLayout* titleLayout;
    private IDWriteTextLayout* detailLayout;
    /// <summary>The live words' wet ones in <see cref="detailLayout"/> (empty unless the text is live words).</summary>
    private DWRITE_TEXT_RANGE wetWords;
    private double dpiScale = 1;
    private bool disposed;
    private DropFallback? fallback;
    private readonly Func<DropFallback> makeFallback;
    /// <summary>Why the fallback window could not be made, while it could not.</summary>
    private string? fallbackFailure;
    /// <summary>What screen readers read for the Drop (its title and detail; the window's title is the state only).</summary>
    private readonly ScreenReaderName speech;

    /// <summary>For tests: an HRESULT to fail the next present with (a lost device), once.</summary>
    internal int FailNextPresent { get; set; }

    /// <summary>The button a press went down on, until it comes up.</summary>
    private int? pressed;

    /// <summary>A button of the text shown was clicked (its index). UI thread.</summary>
    public event Action<int>? ButtonClicked;

    /// <summary>Whether the plain fallback is on screen in the Drop's place.</summary>
    public bool ShowsFallback => fallback?.IsShown == true;

    /// <summary>The fallback window's handle, once made (tests).</summary>
    internal nint FallbackHandle => fallback?.Handle ?? 0;

    /// <summary>The Drop's ink.</summary>
    public InkSurface Surface { get; }

    /// <summary>Whether the Drop is on screen.</summary>
    public bool IsShown { get; private set; }

    /// <summary>What the Drop's lines say now (null while hidden).</summary>
    public DropText? ShownText => IsShown ? text : null;
    private DropText text = new("", "");

    /// <summary>The window's handle (tests: its styles, the foreground window).</summary>
    public nint Handle => (nint)hwnd.Value;

    /// <summary>The pill's colours: the app's mode (the plain fallback follows it too).</summary>
    public DropLook Look
    {
        get => look;
        set
        {
            ArgumentNullException.ThrowIfNull(value);
            if (look == value)
            {
                return;
            }
            look = value;
            fallback?.SetLook(value);
            if (ShowsFallback)
            {
                ShowFallback();
            }
            Surface.Invalidate();
        }
    }

    private DropLook look = DropLook.Default;

    /// <summary>
    /// Creates the (hidden) window on this thread, which must run a message loop: the app's UI
    /// thread. Its Direct3D resources wait for the pipeline.
    /// </summary>
    public DropWindow(InkPipelineLoader loader, InkClock clock)
        : this(loader, clock, () => new DropFallback())
    {
    }

    /// <summary>The same, with the fallback window made by <paramref name="makeFallback"/> (tests make it fail).</summary>
    internal DropWindow(InkPipelineLoader loader, InkClock clock, Func<DropFallback> makeFallback)
    {
        this.makeFallback = makeFallback;
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
                    // An arrow over the buttons, not whatever shape the pointer came in with.
                    hCursor = LoadCursorW(HINSTANCE.NULL, IDC.IDC_ARROW),
                };
                if (RegisterClassExW(&wc) == 0)
                {
                    var error = GetLastError();
                    self.Free();
                    throw new InkRendererException($"couldn't register the Drop's window class (error {error})");
                }
                registered = true;
            }
            hwnd = CreateWindowExW(
                (uint)(WS.WS_EX_NOACTIVATE | WS.WS_EX_TOPMOST | WS.WS_EX_TOOLWINDOW | WS.WS_EX_NOREDIRECTIONBITMAP),
                name, name, unchecked((uint)WS.WS_POPUP), 0, 0, 1, 1, HWND.NULL, HMENU.NULL, (HINSTANCE)instance,
                (void*)GCHandle.ToIntPtr(self));
        }
        if (hwnd == HWND.NULL)
        {
            var error = GetLastError();
            self.Free();
            throw new InkRendererException($"couldn't make the Drop's window (error {error})");
        }
        SetWindowLongPtrW(hwnd, GWLP.GWLP_USERDATA, GCHandle.ToIntPtr(self));
        speech = new ScreenReaderName(hwnd, "the Drop");
        // The fallback is made now, not when the ink first fails: a machine that cannot make it
        // is known (and said) from the start.
        MakeFallback();
        // The recording indicator must never go blank: its device is checked while it shows.
        Surface = new InkSurface(this, loader, clock) { WatchesDevice = true };
        Surface.FailureChanged += _ => ProblemMayHaveChanged();
        ProblemMayHaveChanged();
    }

    /// <summary>
    /// What stands between the user and a working Drop, or null: the ink's failure, and whether the
    /// plain panel could not be made either. The shell shows it where it stays seen while the window
    /// is hidden (the tray icon).
    /// </summary>
    public string? Problem => (Surface?.Failure, fallbackFailure) switch
    {
        (null, null) => null,
        (null, { } f) => $"no plain panel to fall back on: {f}",
        ({ } s, null) => s,
        ({ } s, { } f) => $"{s}; no plain panel either: {f}",
    };

    /// <summary>The problem changed (its new value, or null). UI thread.</summary>
    public event Action<string?>? ProblemChanged;

    private string? lastProblem;

    private void ProblemMayHaveChanged()
    {
        var now = Problem;
        if (now != lastProblem)
        {
            lastProblem = now;
            ProblemChanged?.Invoke(now);
        }
    }

    /// <summary>Makes the fallback window if it is not there yet; a failure is kept and said.</summary>
    private bool MakeFallback()
    {
        if (fallback is not null)
        {
            return true;
        }
        try
        {
            fallback = makeFallback();
            fallback.SetLook(look);
            fallback.ButtonClicked += index => ButtonClicked?.Invoke(index);
            fallbackFailure = null;
        }
        catch (InkRendererException e)
        {
            InkLog.Write(e.Message);
            fallbackFailure = e.Message;
        }
        ProblemMayHaveChanged();
        return fallback is not null;
    }

    /// <summary>Shows the Drop for <paramref name="state"/>, or hides it when nothing is live.</summary>
    public void Update(InkState state)
    {
        if (state.IsLive())
        {
            Show(DropText.For(state), state);
        }
        else
        {
            Hide();
        }
    }

    /// <summary>Shows <paramref name="lines"/> beside the ink in <paramref name="state"/>.</summary>
    public void Show(DropText lines, InkState state)
    {
        ArgumentNullException.ThrowIfNull(lines);
        ObjectDisposedException.ThrowIf(disposed, this);
        var textChanged = lines != text;
        if (textChanged)
        {
            var resized = DropLayout.Size(lines) != DropLayout.Size(text);
            text = lines;
            pressed = null;
            DropLayouts();
            if (resized && IsShown)
            {
                // Buttons came or went: the panel takes its other size where it is.
                Place();
            }
            // The window's title, which any process reads: the title only (see WindowTitle).
            fixed (char* name = lines.WindowTitle)
            {
                SetWindowTextW(hwnd, name);
            }
            if (ShowsFallback)
            {
                ShowFallback();
            }
        }
        // One frame for the change: a new state draws (with the new text); only new text on the
        // same state needs the frame marked out of date.
        if (state != Surface.State)
        {
            Surface.State = state;
        }
        else if (textChanged)
        {
            Surface.Invalidate();
        }
        if (!IsShown)
        {
            Place();
            IsShown = true;
            // SWP_NOACTIVATE: shown, never activated.
            SetWindowPos(hwnd, HWND.HWND_TOPMOST, 0, 0, 0, 0,
                SWP.SWP_NOMOVE | SWP.SWP_NOSIZE | SWP.SWP_NOACTIVATE | SWP.SWP_SHOWWINDOW);
            Surface.SetOnScreen(true);
        }
        // What a screen reader reads, as on the Mac: the title and the detail (live words too),
        // read out politely at each change, once the window shows. Only this window announces; the
        // fallback, over it, holds the same name.
        speech.Set(lines.AccessibleName, announce: true);
    }

    /// <summary>Hides the Drop. Out first: the ink stops without drawing a frame nobody would see.</summary>
    public void Hide()
    {
        if (!IsShown)
        {
            return;
        }
        ShowWindow(hwnd, SW.SW_HIDE);
        IsShown = false;
        // The last words go with the Drop.
        speech.Clear();
        Surface.SetOnScreen(false);
        Surface.State = InkState.Idle;
        // The next show starts from rest, not from the state it was hidden in.
        Surface.Settle();
    }

    /// <summary>
    /// A DPI or display change while shown: the Drop placed and sized again, and its fallback, if
    /// it shows, moved and scaled with it.
    /// </summary>
    internal void DisplayChanged()
    {
        Place();
        Surface.Invalidate();
        if (ShowsFallback)
        {
            ShowFallback();
        }
    }

    /// <summary>
    /// Bottom centre of the work area of the monitor the user is working on (the one with the
    /// foreground window), sized for its DPI.
    /// </summary>
    private void Place()
    {
        var monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR.MONITOR_DEFAULTTOPRIMARY);
        var info = new MONITORINFO { cbSize = (uint)sizeof(MONITORINFO) };
        uint dpiX = 96, dpiY = 96;
        if (!GetMonitorInfoW(monitor, &info) || GetDpiForMonitor(monitor, MONITOR_DPI_TYPE.MDT_EFFECTIVE_DPI, &dpiX, &dpiY).FAILED)
        {
            InkLog.Write("couldn't read the monitor's work area or DPI; placing the Drop on the primary monitor at 100 %");
            info.rcWork = new RECT { left = 0, top = 0, right = GetSystemMetrics(SM.SM_CXSCREEN), bottom = GetSystemMetrics(SM.SM_CYSCREEN) };
            dpiX = 96;
        }
        Layout(dpiX / 96.0);
        var (w, h) = PixelSize();
        var work = info.rcWork;
        var x = work.left + (work.right - work.left - w) / 2;
        var y = work.bottom - (int)Math.Round(DropLayout.BottomMargin * dpiScale) - h;
        SetWindowPos(hwnd, HWND.HWND_TOPMOST, x, y, w, h, SWP.SWP_NOACTIVATE);
    }

    private (int W, int H) PixelSize()
    {
        var (w, h) = DropLayout.Size(text);
        return ((int)Math.Round(w * dpiScale), (int)Math.Round(h * dpiScale));
    }

    /// <summary>Sizes the swapchain and the ink canvas for a DPI scale.</summary>
    private void Layout(double scale)
    {
        dpiScale = scale;
        var (w, h) = PixelSize();
        try
        {
            swapChain?.Resize(w, h);
        }
        catch (InkRendererException e)
        {
            // Reached from Show and from the window procedure, where an exception would end the
            // process: the Drop stops drawing and says why, as for a failed frame.
            Surface.DeviceFailed(e.Message);
            return;
        }
        var (inkW, inkH) = InkSurface.CanvasPixels(DropLayout.OrbSize, DropLayout.OrbSize, scale);
        if (inkTexture is null || inkTexture.Width != inkW || inkTexture.Height != inkH)
        {
            Com.Release(ref inkBitmap);
            inkTexture?.Dispose();
            inkTexture = null;
        }
        Surface.SetCanvas(inkW, inkH);
    }

    void IInkTarget.SetFallback(bool shown)
    {
        if (shown && IsShown)
        {
            ShowFallback();
        }
        else
        {
            fallback?.Hide();
        }
    }

    /// <summary>The plain panel over the Drop's rectangle; the Drop's own window stays, transparent, under it.</summary>
    private void ShowFallback()
    {
        // Tried again each time it is needed.
        if (!MakeFallback())
        {
            return;
        }
        RECT bounds;
        GetWindowRect(hwnd, &bounds);
        fallback!.Show(bounds, text, dpiScale);
    }

    /// <summary>
    /// DWM restarted: the composition device is gone and nothing this window shows would reach the
    /// screen, although every call still succeeds.
    /// </summary>
    string? IInkTarget.CheckDevice()
    {
        if (composition == null)
        {
            return null;
        }
        BOOL valid;
        return composition->CheckDeviceState(&valid).FAILED || !valid
            ? "couldn't keep the Drop's composition device (DWM restarted?)"
            : null;
    }

    /// <summary>Every object made on the lost (or replaced) device. The window stays.</summary>
    void IInkTarget.ReleaseDeviceResources()
    {
        DropLayouts();
        Com.Release(ref titleFormat);
        Com.Release(ref detailFormat);
        Com.Release(ref buttonFormat);
        Com.Release(ref inkBitmap);
        inkTexture?.Dispose();
        inkTexture = null;
        Com.Release(ref visual);
        Com.Release(ref compositionTarget);
        Com.Release(ref composition);
        swapChain?.Dispose();
        swapChain = null;
    }

    /// <summary>The Direct3D, Direct2D and DirectComposition objects, made on first draw.</summary>
    private void EnsureResources(InkPipeline pipeline)
    {
        var (w, h) = PixelSize();
        if (swapChain is null)
        {
            swapChain = new CompositionSwapChain(pipeline, w, h);
            IDCompositionDevice* device;
            InkRendererException.Check(
                DirectX.DCompositionCreateDevice(pipeline.DxgiDevice, __uuidof<IDCompositionDevice>(), (void**)&device),
                "make the Drop's composition device");
            composition = device;
            IDCompositionTarget* target;
            InkRendererException.Check(composition->CreateTargetForHwnd(hwnd, true, &target), "compose into the Drop's window");
            compositionTarget = target;
            IDCompositionVisual* v;
            InkRendererException.Check(composition->CreateVisual(&v), "make the Drop's visual");
            visual = v;
            InkRendererException.Check(visual->SetContent(swapChain.Unknown), "show the Drop's swapchain");
            InkRendererException.Check(compositionTarget->SetRoot(visual), "show the Drop's visual");
            InkRendererException.Check(composition->Commit(), "commit the Drop's composition");
        }
        swapChain.Resize(w, h);
        var (inkW, inkH) = Surface.Canvas;
        if (inkTexture is null)
        {
            inkTexture = new InkTexture(pipeline, inkW, inkH);
            IDXGISurface* surface;
            InkRendererException.Check(inkTexture.Texture->QueryInterface(__uuidof<IDXGISurface>(), (void**)&surface), "reach the ink canvas");
            var props = new D2D1_BITMAP_PROPERTIES1
            {
                pixelFormat = new D2D1_PIXEL_FORMAT { format = InkPipeline.PixelFormat, alphaMode = D2D1_ALPHA_MODE.D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX = 96,
                dpiY = 96,
            };
            ID2D1Bitmap1* b;
            var hr = pipeline.D2D->CreateBitmapFromDxgiSurface(surface, &props, &b);
            surface->Release();
            InkRendererException.Check(hr, "draw the ink on the Drop");
            inkBitmap = b;
        }
        if (titleFormat is null)
        {
            titleFormat = Format(pipeline, DropLayout.Face, DropLayout.TitleSize, DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_SEMI_BOLD, wrap: false);
            detailFormat = Format(pipeline, DropLayout.DetailFace, DropLayout.DetailSize, DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_NORMAL, wrap: true);
            buttonFormat = Format(pipeline, DropLayout.Face, DropLayout.ButtonTextSize, DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_SEMI_BOLD, wrap: false);
        }
        if (titleLayout is null)
        {
            var width = (float)(DropLayout.Size(text).W - DropLayout.TextLeft - DropLayout.TextRight);
            titleLayout = Layout(pipeline, text.Title, titleFormat, width, 100);
            DWRITE_LINE_METRICS line;
            uint count;
            using (var one = new DisposableLayout(Layout(pipeline, "Ag", detailFormat, width, 100)))
            {
                InkRendererException.Check(one.Layout->GetLineMetrics(&line, 1, &count), "measure the Drop's text");
            }
            if (text.LiveWords)
            {
                // One line, the head cut so the newest words show, the last ones wet (italic here;
                // muted as they are drawn).
                var cut = DropText.HeadCut(text.Detail, s => Measure(pipeline, s, detailFormat), width);
                detailLayout = Layout(pipeline, cut, detailFormat, width, line.height + 0.5f);
                detailLayout->SetWordWrapping(DWRITE_WORD_WRAPPING.DWRITE_WORD_WRAPPING_NO_WRAP);
                var wet = DropText.WetStart(cut);
                wetWords = new DWRITE_TEXT_RANGE { startPosition = (uint)wet, length = (uint)(cut.Length - wet) };
                detailLayout->SetFontStyle(DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_ITALIC, wetWords);
            }
            else
            {
                // Two lines at most, the tail cut with an ellipsis.
                detailLayout = Layout(pipeline, text.Detail, detailFormat, width, line.height * 2 + 0.5f);
                wetWords = default;
            }
        }
    }

    /// <summary>The width of <paramref name="s"/> on one line in <paramref name="format"/>, in DIPs.</summary>
    private static double Measure(InkPipeline pipeline, string s, IDWriteTextFormat* format)
    {
        using var layout = new DisposableLayout(Layout(pipeline, s, format, 100_000, 100));
        layout.Layout->SetWordWrapping(DWRITE_WORD_WRAPPING.DWRITE_WORD_WRAPPING_NO_WRAP);
        DWRITE_TEXT_METRICS metrics;
        InkRendererException.Check(layout.Layout->GetMetrics(&metrics), "measure the Drop's text");
        return metrics.widthIncludingTrailingWhitespace;
    }

    private static IDWriteTextFormat* Format(InkPipeline pipeline, string face, float size, DWRITE_FONT_WEIGHT weight, bool wrap)
    {
        IDWriteTextFormat* format;
        fixed (char* family = face)
        fixed (char* locale = "en-us")
        {
            InkRendererException.Check(pipeline.DWrite->CreateTextFormat(family, null, weight,
                DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH.DWRITE_FONT_STRETCH_NORMAL, size, locale, &format),
                "make the Drop's text format");
        }
        format->SetWordWrapping(wrap ? DWRITE_WORD_WRAPPING.DWRITE_WORD_WRAPPING_WRAP : DWRITE_WORD_WRAPPING.DWRITE_WORD_WRAPPING_NO_WRAP);
        IDWriteInlineObject* ellipsis;
        if (pipeline.DWrite->CreateEllipsisTrimmingSign(format, &ellipsis).SUCCEEDED)
        {
            var trimming = new DWRITE_TRIMMING { granularity = DWRITE_TRIMMING_GRANULARITY.DWRITE_TRIMMING_GRANULARITY_CHARACTER };
            format->SetTrimming(&trimming, ellipsis);
            ellipsis->Release();
        }
        return format;
    }

    private static IDWriteTextLayout* Layout(InkPipeline pipeline, string s, IDWriteTextFormat* format, float width, float height)
    {
        IDWriteTextLayout* layout;
        fixed (char* chars = s)
        {
            InkRendererException.Check(pipeline.DWrite->CreateTextLayout(chars, (uint)s.Length, format, width, height, &layout),
                "lay out the Drop's text");
        }
        return layout;
    }

    private readonly struct DisposableLayout(IDWriteTextLayout* layout) : IDisposable
    {
        public IDWriteTextLayout* Layout => layout;
        public void Dispose() => layout->Release();
    }

    private void DropLayouts()
    {
        Com.Release(ref titleLayout);
        Com.Release(ref detailLayout);
    }

    /// <summary>One frame: the orb offscreen, then the pill on the swapchain, then present.</summary>
    bool IInkTarget.Render(InkPipeline pipeline, in InkUniforms uniforms)
    {
        if (!IsShown)
        {
            return false;
        }
        EnsureResources(pipeline);
        if (((IInkTarget)this).CheckDevice() is { } lostComposition)
        {
            throw new InkRendererException(lostComposition);
        }
        pipeline.Encode(inkTexture!.View, inkTexture.Width, inkTexture.Height, uniforms);

        var d2d = pipeline.D2D;
        d2d->SetTarget((ID2D1Image*)swapChain!.TargetBitmap);
        var dpi = (float)(96 * dpiScale);
        d2d->SetDpi(dpi, dpi);
        d2d->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE.D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        d2d->BeginDraw();
        ID2D1SolidColorBrush* fill = null, border = null, title = null, detail = null, wet = null, button = null, label = null;
        ID2D1RoundedRectangleGeometry* panel = null;
        ID2D1EllipseGeometry* circle = null;
        HRESULT hr;
        try
        {
            var clear = new DXGI_RGBA();
            d2d->Clear(&clear);
            var (panelW, panelH) = DropLayout.Size(text);
            var bounds = new D2D1_ROUNDED_RECT
            {
                rect = new D2D_RECT_F { left = 0, top = 0, right = (float)panelW, bottom = (float)panelH },
                radiusX = (float)DropLayout.CornerRadius,
                radiusY = (float)DropLayout.CornerRadius,
            };
            InkRendererException.Check(pipeline.D2DFactory->CreateRoundedRectangleGeometry(&bounds, &panel), "shape the Drop");
            fill = Brush(d2d, look.Background, 1);
            d2d->FillGeometry((ID2D1Geometry*)panel, (ID2D1Brush*)fill, null);

            // The orb in its circle, which keeps its size, centred on the pill's first height
            // (the offer's pill grows below it).
            var orbRect = new D2D_RECT_F
            {
                left = (float)DropLayout.OrbInset,
                top = (float)DropLayout.OrbInset,
                right = (float)(DropLayout.OrbInset + DropLayout.OrbSize),
                bottom = (float)(DropLayout.OrbInset + DropLayout.OrbSize),
            };
            var ellipse = new D2D1_ELLIPSE
            {
                point = new D2D_POINT_2F((orbRect.left + orbRect.right) / 2, (orbRect.top + orbRect.bottom) / 2),
                radiusX = (float)DropLayout.OrbSize / 2,
                radiusY = (float)DropLayout.OrbSize / 2,
            };
            InkRendererException.Check(pipeline.D2DFactory->CreateEllipseGeometry(&ellipse, &circle), "shape the Drop's orb");
            var layer = new D2D1_LAYER_PARAMETERS1
            {
                contentBounds = orbRect,
                geometricMask = (ID2D1Geometry*)circle,
                maskAntialiasMode = D2D1_ANTIALIAS_MODE.D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform = new D2D_MATRIX_3X2_F { _11 = 1, _22 = 1 },
                opacity = 1,
                layerOptions = D2D1_LAYER_OPTIONS1.D2D1_LAYER_OPTIONS1_NONE,
            };
            d2d->PushLayer(&layer, null);
            d2d->DrawBitmap((ID2D1Bitmap*)inkBitmap, &orbRect, 1, D2D1_INTERPOLATION_MODE.D2D1_INTERPOLATION_MODE_LINEAR, null, null);
            d2d->PopLayer();

            // The hairline, inside the pill's edge; the alert colour and heavier for an alert.
            var alert = text.Tone == DropTone.Alert;
            var width = alert ? 1.5f : 1f;
            border = Brush(d2d, alert ? look.Alert : look.Border, 1);
            var inset = new D2D1_ROUNDED_RECT
            {
                rect = new D2D_RECT_F { left = width / 2, top = width / 2, right = (float)panelW - width / 2, bottom = (float)panelH - width / 2 },
                radiusX = (float)DropLayout.CornerRadius - width / 2,
                radiusY = (float)DropLayout.CornerRadius - width / 2,
            };
            d2d->DrawRoundedRectangle(&inset, (ID2D1Brush*)border, width, null);

            // The two lines, stacked and centred on the pill's height; text never takes a dot colour.
            title = Brush(d2d, text.Tone == DropTone.Plain ? look.Secondary : look.Alert, 1);
            detail = Brush(d2d, look.Text, 1);
            if (wetWords.length > 0)
            {
                // The newest live words in the secondary colour (a brush lives on the device, so it
                // is set per frame).
                wet = Brush(d2d, look.Secondary, 1);
                detailLayout->SetDrawingEffect((IUnknown*)wet, wetWords);
            }
            DWRITE_TEXT_METRICS tm, dm;
            titleLayout->GetMetrics(&tm);
            detailLayout->GetMetrics(&dm);
            var total = tm.height + DropLayout.LineSpacing + dm.height;
            // Above the buttons when there are some, else centred on the pill.
            var linesHeight = text.Buttons is null ? panelH : DropLayout.Button(0).Top - 4;
            var top = (linesHeight - total) / 2;
            d2d->DrawTextLayout(new D2D_POINT_2F((float)DropLayout.TextLeft, (float)top), titleLayout, (ID2D1Brush*)title,
                D2D1_DRAW_TEXT_OPTIONS.D2D1_DRAW_TEXT_OPTIONS_NONE);
            d2d->DrawTextLayout(new D2D_POINT_2F((float)DropLayout.TextLeft, (float)(top + tm.height + DropLayout.LineSpacing)),
                detailLayout, (ID2D1Brush*)detail, D2D1_DRAW_TEXT_OPTIONS.D2D1_DRAW_TEXT_OPTIONS_NONE);

            if (text.Buttons is { } buttons)
            {
                // Pills: the first in the button fill (the answer), the second outlined in the text colour.
                button = Brush(d2d, look.ButtonFill, 1);
                label = Brush(d2d, look.ButtonLabel, 1);
                for (var i = 0; i < buttons.Count; i++)
                {
                    var (left, btop, right, bottom) = DropLayout.Button(i);
                    var radius = (float)DropLayout.ButtonHeight / 2;
                    var shape = new D2D1_ROUNDED_RECT
                    {
                        rect = new D2D_RECT_F { left = (float)left + 0.5f, top = (float)btop + 0.5f, right = (float)right - 0.5f, bottom = (float)bottom - 0.5f },
                        radiusX = radius,
                        radiusY = radius,
                    };
                    if (i == 0)
                    {
                        d2d->FillRoundedRectangle(&shape, (ID2D1Brush*)button);
                    }
                    else
                    {
                        d2d->DrawRoundedRectangle(&shape, (ID2D1Brush*)detail, 1, null);
                    }
                    var words = buttonFormat is null ? null : Layout(pipeline, buttons[i], buttonFormat, (float)(right - left), (float)(bottom - btop));
                    if (words != null)
                    {
                        words->SetTextAlignment(DWRITE_TEXT_ALIGNMENT.DWRITE_TEXT_ALIGNMENT_CENTER);
                        words->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT.DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
                        d2d->DrawTextLayout(new D2D_POINT_2F((float)left, (float)btop), words, (ID2D1Brush*)(i == 0 ? label : detail),
                            D2D1_DRAW_TEXT_OPTIONS.D2D1_DRAW_TEXT_OPTIONS_NONE);
                        words->Release();
                    }
                }
            }
        }
        finally
        {
            hr = d2d->EndDraw(null, null);
            d2d->SetTarget(null);
            if (wet != null)
            {
                // The layout keeps no brush past the frame that drew it.
                detailLayout->SetDrawingEffect(null, wetWords);
            }
            Com.Release(ref wet);
            Com.Release(ref label);
            Com.Release(ref button);
            Com.Release(ref detail);
            Com.Release(ref title);
            Com.Release(ref border);
            Com.Release(ref fill);
            Com.Release(ref circle);
            Com.Release(ref panel);
        }
        InkRendererException.Check(hr, "draw the Drop");
        swapChain.InjectedPresentResult = FailNextPresent;
        FailNextPresent = 0;
        swapChain.Present();
        return true;
    }

    private static ID2D1SolidColorBrush* Brush(ID2D1DeviceContext* d2d, (float R, float G, float B) c, float alpha)
    {
        var colour = new DXGI_RGBA { r = c.R, g = c.G, b = c.B, a = alpha };
        ID2D1SolidColorBrush* brush;
        InkRendererException.Check(d2d->CreateSolidColorBrush(&colour, null, &brush), "make the Drop's brush");
        return brush;
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
            InkLog.Write($"the Drop's window procedure failed (message 0x{msg:X4}): {e.GetType().Name}: {e.Message}");
            return DefWindowProcW(hwnd, msg, wParam, lParam);
        }
    }

    private static LRESULT Dispatch(HWND hwnd, uint msg, WPARAM wParam, LPARAM lParam)
    {
        switch (msg)
        {
            case WM.WM_MOUSEACTIVATE:
                // A click on the Drop never activates it (or Inkwell).
                return MA.MA_NOACTIVATE;
            case WM.WM_LBUTTONDOWN:
            case WM.WM_LBUTTONUP:
                if (From(hwnd) is { IsShown: true } clicked)
                {
                    clicked.Mouse(msg == WM.WM_LBUTTONDOWN, lParam);
                    return 0;
                }
                break;
            case WM.WM_SETTINGCHANGE:
                // Every top-level window hears it, hidden ones included: Animation effects may
                // have changed.
                SystemMotion.SettingChanged();
                break;
            case WM.WM_DPICHANGED:
            case WM.WM_DISPLAYCHANGE:
                if (From(hwnd) is { IsShown: true } drop)
                {
                    drop.DisplayChanged();
                }
                break;
            default:
                break;
        }
        return DefWindowProcW(hwnd, msg, wParam, lParam);
    }

    /// <summary>A left button went down or up at <paramref name="lParam"/> (client pixels).</summary>
    private void Mouse(bool down, LPARAM lParam) => Press(down, ClientDips(lParam));

    /// <summary>The client point in <paramref name="lParam"/>, in DIPs.</summary>
    internal (double X, double Y) ClientDips(LPARAM lParam)
    {
        var x = (short)((nint)lParam & 0xFFFF);
        var y = (short)(((nint)lParam >> 16) & 0xFFFF);
        return (x / dpiScale, y / dpiScale);
    }

    /// <summary>A press at <paramref name="at"/> (DIPs): a click is down and up on the same button.</summary>
    internal void Press(bool down, (double X, double Y) at)
    {
        var button = DropLayout.ButtonAt(text.Buttons, at.X, at.Y);
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

    private static DropWindow? From(HWND hwnd)
    {
        var handle = GetWindowLongPtrW(hwnd, GWLP.GWLP_USERDATA);
        return handle == 0 ? null : GCHandle.FromIntPtr(handle).Target as DropWindow;
    }

    /// <summary>Hides and destroys the window and releases its resources. UI thread.</summary>
    public void Dispose()
    {
        if (disposed)
        {
            return;
        }
        disposed = true;
        Surface.Dispose();
        ((IInkTarget)this).ReleaseDeviceResources();
        fallback?.Dispose();
        fallback = null;
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
