// The INKWELL wordmark, rasterised with DirectWrite (through Direct2D) the way the prototype's
// `_uploadMark` does with a canvas and the Mac with CoreText (mac/Sources/InkRenderer/Wordmark.swift):
// coverage only, the size of the canvas, row 0 at the top. The shader knocks it out of whatever
// sits under it: dark on paper, light on ink.
//
// The prototype asks for a 900-weight face (Geist, under the SIL OFL, which is not on this
// project's licence allowlist). The Mac uses its heaviest system face, SF Pro Black; Windows uses
// its heaviest, Segoe UI Black (Segoe UI Variable stops at Bold). The placement is the
// prototype's: size, left edge, and a baseline where a browser canvas puts
// `textBaseline = 'top'`.
using TerraFX.Interop.DirectX;
using TerraFX.Interop.Windows;
using static TerraFX.Interop.DirectX.DXGI_FORMAT;

namespace Inkwell.Ink;

/// <summary>A rectangle in canvas pixels from the top left.</summary>
public readonly record struct InkRect(double X, double Y, double Width, double Height)
{
    /// <summary>Whether the point is inside.</summary>
    public bool Contains(double x, double y) => x >= X && x < X + Width && y >= Y && y < Y + Height;

    /// <summary>The smallest rectangle holding both.</summary>
    public InkRect Union(InkRect o)
    {
        double x0 = Math.Min(X, o.X), y0 = Math.Min(Y, o.Y);
        double x1 = Math.Max(X + Width, o.X + o.Width), y1 = Math.Max(Y + Height, o.Y + o.Height);
        return new InkRect(x0, y0, x1 - x0, y1 - y0);
    }

    /// <summary>Grown by <paramref name="d"/> on every side.</summary>
    public InkRect Outset(double d) => new(X - d, Y - d, Width + 2 * d, Height + 2 * d);
}

/// <summary>The rasterised wordmark: its texture and where the letters sit.</summary>
public sealed unsafe class Wordmark : IDisposable
{
    /// <summary>The heaviest system face: Segoe UI Black.</summary>
    public const string SystemFamily = "Segoe UI";
    /// <summary>Black, 900.</summary>
    public const int SystemWeight = 900;

    /// <summary>The coverage texture, the canvas's size.</summary>
    public InkMark Mark { get; }
    /// <summary>The face DirectWrite used (PostScript name).</summary>
    public string FontName { get; }
    /// <summary>Whether DirectWrite had to synthesise bold or oblique (a face missing its weight).</summary>
    public bool Simulated { get; }
    /// <summary>The size in pixels.</summary>
    public double FontSize { get; }
    /// <summary>The text's left edge.</summary>
    public double X { get; }
    /// <summary>The baseline, from the top.</summary>
    public double BaselineFromTop { get; }
    /// <summary>The advance width.</summary>
    public double TextWidth { get; }
    /// <summary>The box the letters can occupy (advance width by ascent and descent).</summary>
    public InkRect Box { get; }

    private Wordmark(InkMark mark, string fontName, bool simulated, double fontSize, double x, double baseline, double textWidth, InkRect box)
    {
        Mark = mark;
        FontName = fontName;
        Simulated = simulated;
        FontSize = fontSize;
        X = x;
        BaselineFromTop = baseline;
        TextWidth = textWidth;
        Box = box;
    }

    /// <summary>
    /// The wordmark for a canvas of <paramref name="width"/> x <paramref name="height"/> pixels drawn
    /// at <paramref name="pointWidth"/> DIPs wide: the prototype sizes it from the canvas width in
    /// CSS pixels and the scale. UI thread.
    /// </summary>
    public static Wordmark Rasterize(InkPipeline pipeline, int width, int height, double pointWidth,
        string family = SystemFamily, int weight = SystemWeight)
    {
        var scale = width / pointWidth;
        var fontSize = JsRound(pointWidth * 0.158 * scale);
        var x = JsRound(fontSize * 0.62);
        var top = JsRound(fontSize * 0.7);
        var face = Face.Find(pipeline, family, (DWRITE_FONT_WEIGHT)weight);
        // Skia snaps an axis-aligned baseline to whole pixels; so does this (and the Mac).
        var baseline = Math.Round(top + face.EmBoxAscent(fontSize), MidpointRounding.AwayFromZero);

        IDWriteTextFormat* format = null;
        IDWriteTextLayout* layout = null;
        ID2D1Bitmap1* target = null;
        IDXGISurface* surface = null;
        ID2D1SolidColorBrush* brush = null;
        var mark = InkMark.Create(pipeline, width, height, renderTarget: true, ReadOnlySpan<byte>.Empty);
        try
        {
            fixed (char* fam = family)
            fixed (char* locale = "en-us")
            {
                InkRendererException.Check(pipeline.DWrite->CreateTextFormat(fam, null, (DWRITE_FONT_WEIGHT)weight,
                    DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH.DWRITE_FONT_STRETCH_NORMAL,
                    (float)fontSize, locale, &format), "make the wordmark's text format");
            }
            fixed (char* text = "INKWELL")
            {
                InkRendererException.Check(pipeline.DWrite->CreateTextLayout(text, 7, format, 100_000, 100_000, &layout),
                    "lay out the wordmark");
            }
            DWRITE_LINE_METRICS line;
            uint lines;
            InkRendererException.Check(layout->GetLineMetrics(&line, 1, &lines), "measure the wordmark's line");
            DWRITE_TEXT_METRICS metrics;
            InkRendererException.Check(layout->GetMetrics(&metrics), "measure the wordmark");

            InkRendererException.Check(mark.Texture->QueryInterface(Windows.__uuidof<IDXGISurface>(), (void**)&surface),
                "reach the wordmark texture's surface");
            var props = new D2D1_BITMAP_PROPERTIES1
            {
                pixelFormat = new D2D1_PIXEL_FORMAT { format = DXGI_FORMAT_A8_UNORM, alphaMode = D2D1_ALPHA_MODE.D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX = 96,
                dpiY = 96,
                bitmapOptions = D2D1_BITMAP_OPTIONS.D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS.D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            };
            var d2d = pipeline.D2D;
            InkRendererException.Check(d2d->CreateBitmapFromDxgiSurface(surface, &props, &target), "draw into the wordmark texture");
            var black = new DXGI_RGBA { r = 0, g = 0, b = 0, a = 1 };
            InkRendererException.Check(d2d->CreateSolidColorBrush(&black, null, &brush), "make the wordmark's brush");
            d2d->SetTarget((ID2D1Image*)target);
            d2d->SetDpi(96, 96);
            d2d->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE.D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            d2d->BeginDraw();
            var clear = new DXGI_RGBA { r = 0, g = 0, b = 0, a = 0 };
            d2d->Clear(&clear);
            d2d->DrawTextLayout(new D2D_POINT_2F((float)x, (float)(baseline - line.baseline)), layout,
                (ID2D1Brush*)brush, D2D1_DRAW_TEXT_OPTIONS.D2D1_DRAW_TEXT_OPTIONS_NONE);
            var hr = d2d->EndDraw(null, null);
            d2d->SetTarget(null);
            InkRendererException.Check(hr, "draw the wordmark");

            var textWidth = (double)metrics.widthIncludingTrailingWhitespace;
            var box = new InkRect(x, baseline - face.Ascent(fontSize), textWidth, face.Ascent(fontSize) + face.Descent(fontSize));
            return new Wordmark(mark, face.PostScriptName, face.Simulated, fontSize, x, baseline, textWidth, box);
        }
        catch
        {
            mark.Dispose();
            throw;
        }
        finally
        {
            Com.Release(ref brush);
            Com.Release(ref target);
            Com.Release(ref surface);
            Com.Release(ref layout);
            Com.Release(ref format);
        }
    }

    /// <summary>JavaScript's Math.round: floor(x + 0.5).</summary>
    internal static double JsRound(double x) => Math.Floor(x + 0.5);

    /// <summary>Releases the texture.</summary>
    public void Dispose() => Mark.Dispose();

    /// <summary>The face DirectWrite picks for a family and weight, and its vertical metrics.</summary>
    private readonly struct Face
    {
        public string PostScriptName { get; init; }
        public bool Simulated { get; init; }
        private double UnitsPerEm { get; init; }
        private double AscentUnits { get; init; }
        private double DescentUnits { get; init; }
        private double TypoAscender { get; init; }
        private double TypoDescender { get; init; }
        private bool HasTypo { get; init; }

        public double Ascent(double size) => AscentUnits / UnitsPerEm * size;

        public double Descent(double size) => DescentUnits / UnitsPerEm * size;

        /// <summary>
        /// Where a browser canvas's <c>textBaseline = 'top'</c> sits above the baseline: the em
        /// box's top, the OS/2 typo ascender scaled so that ascender plus descender make one em,
        /// rounded to 1/64 px (Chrome's layout unit). Without an OS/2 table, the font's ascent and
        /// descent. As the Mac's Wordmark.emBoxAscent.
        /// </summary>
        public double EmBoxAscent(double size)
        {
            double ascent = Ascent(size), descent = Descent(size);
            if (HasTypo)
            {
                ascent = TypoAscender / UnitsPerEm * size;
                descent = -TypoDescender / UnitsPerEm * size;
            }
            var height = ascent + descent;
            if (height <= 0 || ascent < 0 || ascent > height)
            {
                return size * 0.8;
            }
            return Math.Round(ascent * size / height * 64, MidpointRounding.AwayFromZero) / 64;
        }

        public static Face Find(InkPipeline pipeline, string family, DWRITE_FONT_WEIGHT weight)
        {
            IDWriteFontCollection* collection = null;
            IDWriteFontFamily* fontFamily = null;
            IDWriteFont* font = null;
            IDWriteFontFace* face = null;
            IDWriteLocalizedStrings* names = null;
            try
            {
                InkRendererException.Check(pipeline.DWrite->GetSystemFontCollection(&collection, false), "read the system fonts");
                uint index;
                BOOL exists;
                fixed (char* name = family)
                {
                    InkRendererException.Check(collection->FindFamilyName(name, &index, &exists), "look up the wordmark's font");
                }
                if (!exists)
                {
                    throw new InkRendererException($"couldn't find the font family {family}");
                }
                InkRendererException.Check(collection->GetFontFamily(index, &fontFamily), "open the wordmark's font family");
                InkRendererException.Check(fontFamily->GetFirstMatchingFont(weight, DWRITE_FONT_STRETCH.DWRITE_FONT_STRETCH_NORMAL,
                    DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_NORMAL, &font), "match the wordmark's font");
                DWRITE_FONT_METRICS metrics;
                font->GetMetrics(&metrics);
                var simulated = font->GetSimulations() != DWRITE_FONT_SIMULATIONS.DWRITE_FONT_SIMULATIONS_NONE;
                var postScript = family;
                BOOL has;
                if (font->GetInformationalStrings(DWRITE_INFORMATIONAL_STRING_ID.DWRITE_INFORMATIONAL_STRING_POSTSCRIPT_NAME, &names, &has).SUCCEEDED
                    && has && names != null)
                {
                    uint length;
                    if (names->GetStringLength(0, &length).SUCCEEDED)
                    {
                        var buffer = new char[length + 1];
                        fixed (char* b = buffer)
                        {
                            if (names->GetString(0, b, length + 1).SUCCEEDED)
                            {
                                postScript = new string(b, 0, (int)length);
                            }
                        }
                    }
                }
                InkRendererException.Check(font->CreateFontFace(&face), "open the wordmark's font face");
                // OS/2: sTypoAscender at byte 68, sTypoDescender at 70, both big-endian int16.
                const uint os2 = 'O' | ('S' << 8) | ('/' << 16) | ((uint)'2' << 24);
                void* data;
                uint size;
                void* context;
                BOOL found;
                double typoAscender = 0, typoDescender = 0;
                var hasTypo = false;
                if (face->TryGetFontTable(os2, &data, &size, &context, &found).SUCCEEDED && found)
                {
                    if (size >= 72)
                    {
                        var table = new ReadOnlySpan<byte>(data, (int)size);
                        typoAscender = System.Buffers.Binary.BinaryPrimitives.ReadInt16BigEndian(table[68..]);
                        typoDescender = System.Buffers.Binary.BinaryPrimitives.ReadInt16BigEndian(table[70..]);
                        hasTypo = true;
                    }
                    face->ReleaseFontTable(context);
                }
                return new Face
                {
                    PostScriptName = postScript,
                    Simulated = simulated,
                    UnitsPerEm = metrics.designUnitsPerEm,
                    AscentUnits = metrics.ascent,
                    DescentUnits = metrics.descent,
                    TypoAscender = typoAscender,
                    TypoDescender = typoDescender,
                    HasTypo = hasTypo,
                };
            }
            finally
            {
                Com.Release(ref names);
                Com.Release(ref face);
                Com.Release(ref font);
                Com.Release(ref fontFamily);
                Com.Release(ref collection);
            }
        }
    }
}
