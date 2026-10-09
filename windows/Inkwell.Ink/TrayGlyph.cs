// The live icon's pictures on Windows (the Mac's StatusGlyph and LiveIconDock), drawn in code from
// the app's own icon (Assets/Inkwell.ico, Halo rim: the orb on night inside a glowing rim):
//
//   the tray icon   at rest the icon as it is; a live state keeps its plate and rim and puts one
//                   dot in the orb's place, in the state's colour (the pulse breathes it toward
//                   its lighter tint, never toward the night plate, which turned coral brown, as
//                   the Mac's menu bar found); the final pass fills the rim clockwise from the
//                   top, dashed while it has no number yet
//   the badge       the taskbar button's overlay: the dot alone, ringed in night so it reads on a
//                   light or dark taskbar
//   the thumbnail   the taskbar preview's Record (a dot) and Stop (a square) buttons
//
// No icon files are added: the colours are the user's, so the pictures are drawn when the colours,
// the state or the breath change (never on a timer of their own). Each pixel is 4 x 4 samples, so
// the small sizes stay smooth. The caller owns each HICON (Destroy).
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

/// <summary>What the tray icon shows.</summary>
public enum GlyphLook
{
    /// <summary>The icon as it is.</summary>
    Rest,
    /// <summary>A dot in the orb's place.</summary>
    Dot,
    /// <summary>The rim filled to a progress, or dashed.</summary>
    Ring,
}

public static unsafe class TrayGlyph
{
    /// <summary>The icon's night plate (design/icon/make_icon.py's NIGHT).</summary>
    internal static readonly (float R, float G, float B) Night = Rgb.Of(0x121118);

    /// <summary>How far toward white the breath's faintest is (the colour's lighter tint).</summary>
    internal const float TintLift = 0.5f;

    /// <summary>The dot's colour at <paramref name="strength"/>: the colour itself at 1, toward its lighter tint as the pulse breathes out.</summary>
    internal static (float R, float G, float B) Breathed((float R, float G, float B) colour, double strength) =>
        Rgb.Mix(Rgb.Lift(colour, TintLift), colour, (float)Math.Clamp(strength, 0, 1));

    /// <summary>
    /// The orb's place in the icon, as shares of its side: the small art's discs (centre 50, 51 of
    /// 100, out to 27 across both), the dot drawn there, and the rim's centre line and corner.
    /// </summary>
    internal const double OrbX = 0.5;
    internal const double OrbY = 0.51;
    internal const double OrbCover = 0.30;
    internal const double DotRadius = 0.22;
    internal const double RimInset = 0.06;
    internal const double RimCorner = 0.17;

    /// <summary>
    /// The tray icon at the small-icon size for <paramref name="look"/>: <paramref name="colour"/>
    /// at <paramref name="strength"/> for a dot, or the ring's <paramref name="progress"/> (null:
    /// dashed) in it. Throws <see cref="InkRendererException"/> when the icon cannot be read or made.
    /// </summary>
    public static nint Tray(string path, GlyphLook look, (float R, float G, float B) colour, double strength, double? progress)
    {
        ArgumentNullException.ThrowIfNull(path);
        var size = GetSystemMetrics(SM.SM_CXSMICON);
        var pixels = Load(path, size);
        Paint(pixels, size, look, colour, strength, progress);
        return ToIcon(pixels, size);
    }

    /// <summary>Paints <paramref name="look"/> over the icon's pixels (BGRA, straight alpha, row 0 at the top).</summary>
    internal static void Paint(byte[] bgra, int size, GlyphLook look, (float R, float G, float B) colour, double strength, double? progress)
    {
        switch (look)
        {
            case GlyphLook.Dot:
                // The orb's place painted over with the plate, then the dot, breathed.
                Disc(bgra, size, OrbX, OrbY, OrbCover, Night, 1);
                Disc(bgra, size, OrbX, OrbY, DotRadius, Breathed(colour, strength), 1);
                break;
            case GlyphLook.Ring:
                PaintRing(bgra, size, colour, progress);
                break;
            default:
                break;
        }
    }

    /// <summary>The taskbar badge: a dot of <paramref name="colour"/> at <paramref name="strength"/>, ringed in night.</summary>
    public static nint Badge((float R, float G, float B) colour, double strength)
    {
        var size = GetSystemMetrics(SM.SM_CXSMICON);
        var pixels = new byte[size * size * 4];
        Disc(pixels, size, 0.5, 0.5, 0.5, Night, 1);
        Disc(pixels, size, 0.5, 0.5, 0.36, Breathed(colour, strength), 1);
        return ToIcon(pixels, size);
    }

    /// <summary>A thumbnail button's picture: Record's dot or Stop's rounded square, in <paramref name="colour"/>.</summary>
    public static nint Thumb(bool stop, (float R, float G, float B) colour)
    {
        var size = GetSystemMetrics(SM.SM_CXSMICON);
        var pixels = new byte[size * size * 4];
        if (stop)
        {
            Shape(pixels, size, colour, 1, (x, y) => RoundedRectDistance(x, y, 0.22, 0.12) <= 0);
        }
        else
        {
            Disc(pixels, size, 0.5, 0.5, 0.34, colour, 1);
        }
        return ToIcon(pixels, size);
    }

    /// <summary>Releases an icon made here.</summary>
    public static void Destroy(nint icon)
    {
        if (icon != 0)
        {
            DestroyIcon((HICON)icon);
        }
    }

    /// <summary>
    /// The rim, at least a pixel and a half wide: a faint track all the way round, then
    /// <paramref name="colour"/> clockwise from the top to <paramref name="progress"/>, or in even
    /// dashes with no number, a dash for every two pixels of the side so each stays a pixel or more
    /// at the tray's 16 (still: an indeterminate ring that spun would be motion the icon never has).
    /// </summary>
    private static void PaintRing(byte[] bgra, int size, (float R, float G, float B) colour, double? progress)
    {
        var width = Math.Max(1.5 / size, 0.09);
        Shape(bgra, size, (1, 1, 1), 0.22, (x, y) => OnRim(x, y, width));
        Shape(bgra, size, colour, 1, (x, y) =>
        {
            if (!OnRim(x, y, width))
            {
                return false;
            }
            // Clockwise from the top, as a clock's hand: 0 at twelve, 1 all the way round.
            var turn = (Math.Atan2(x - 0.5, 0.5 - y) / (2 * Math.PI) + 1) % 1;
            return progress is double p ? turn < Math.Clamp(p, 0, 1) : (int)(turn * size) % 2 == 0;
        });
    }

    /// <summary>Whether (x, y), shares of the side, lies on the rim's stroke of <paramref name="width"/>.</summary>
    private static bool OnRim(double x, double y, double width) => Math.Abs(RoundedRectDistance(x, y, RimInset, RimCorner)) <= width / 2;

    /// <summary>Signed distance from (x, y) to a rounded square inset by <paramref name="inset"/> with corners of <paramref name="corner"/> (negative inside).</summary>
    private static double RoundedRectDistance(double x, double y, double inset, double corner)
    {
        var half = 0.5 - inset;
        var qx = Math.Abs(x - 0.5) - (half - corner);
        var qy = Math.Abs(y - 0.5) - (half - corner);
        var outside = Math.Sqrt(Math.Pow(Math.Max(qx, 0), 2) + Math.Pow(Math.Max(qy, 0), 2));
        return outside + Math.Min(Math.Max(qx, qy), 0) - corner;
    }

    /// <summary>A disc of <paramref name="radius"/> (shares of the side) at (cx, cy) in <paramref name="colour"/> at <paramref name="alpha"/>.</summary>
    private static void Disc(byte[] bgra, int size, double cx, double cy, double radius, (float R, float G, float B) colour, double alpha) =>
        Shape(bgra, size, colour, alpha, (x, y) => (x - cx) * (x - cx) + (y - cy) * (y - cy) <= radius * radius);

    /// <summary>
    /// <paramref name="colour"/> at <paramref name="alpha"/> wherever <paramref name="inside"/> says,
    /// over what is there (straight alpha), antialiased by 4 x 4 samples per pixel.
    /// </summary>
    private static void Shape(byte[] bgra, int size, (float R, float G, float B) colour, double alpha, Func<double, double, bool> inside)
    {
        for (var y = 0; y < size; y++)
        {
            for (var x = 0; x < size; x++)
            {
                var cover = 0;
                for (var sy = 0; sy < 4; sy++)
                {
                    for (var sx = 0; sx < 4; sx++)
                    {
                        if (inside((x + (sx + 0.5) / 4) / size, (y + (sy + 0.5) / 4) / size))
                        {
                            cover++;
                        }
                    }
                }
                if (cover == 0)
                {
                    continue;
                }
                var a = cover / 16.0 * alpha;
                var i = (y * size + x) * 4;
                var under = bgra[i + 3] / 255.0;
                var outA = a + under * (1 - a);
                if (outA <= 0)
                {
                    continue;
                }
                bgra[i] = Channel((colour.B * a + bgra[i] / 255.0 * under * (1 - a)) / outA);
                bgra[i + 1] = Channel((colour.G * a + bgra[i + 1] / 255.0 * under * (1 - a)) / outA);
                bgra[i + 2] = Channel((colour.R * a + bgra[i + 2] / 255.0 * under * (1 - a)) / outA);
                bgra[i + 3] = Channel(outA);
            }
        }
    }

    private static byte Channel(double v) => (byte)Math.Round(Math.Clamp(v, 0, 1) * 255);

    /// <summary>The icon at <paramref name="path"/> at <paramref name="size"/> pixels, as BGRA (straight alpha, row 0 at the top).</summary>
    internal static byte[] Load(string path, int size)
    {
        HICON source;
        fixed (char* file = path)
        {
            source = (HICON)LoadImageW(HINSTANCE.NULL, file, IMAGE.IMAGE_ICON, size, size, LR.LR_LOADFROMFILE);
        }
        if (source == HICON.NULL)
        {
            throw new InkRendererException($"couldn't load the tray icon (error {GetLastError()})");
        }
        ICONINFO info;
        var read = GetIconInfo(source, &info);
        DestroyIcon(source);
        if (!read)
        {
            throw new InkRendererException($"couldn't read the tray icon (error {GetLastError()})");
        }
        var screen = GetDC(HWND.NULL);
        try
        {
            var header = Header(size);
            var pixels = new byte[size * size * 4];
            fixed (byte* bgra = pixels)
            {
                if (info.hbmColor == HBITMAP.NULL || GetDIBits(screen, info.hbmColor, 0, (uint)size, bgra, &header, DIB_RGB_COLORS) == 0)
                {
                    throw new InkRendererException("couldn't read the tray icon's pixels");
                }
            }
            return pixels;
        }
        finally
        {
            if (info.hbmColor != HBITMAP.NULL)
            {
                DeleteObject((HGDIOBJ)info.hbmColor.Value);
            }
            if (info.hbmMask != HBITMAP.NULL)
            {
                DeleteObject((HGDIOBJ)info.hbmMask.Value);
            }
            _ = ReleaseDC(HWND.NULL, screen);
        }
    }

    private static BITMAPINFO Header(int size) => new()
    {
        bmiHeader = new BITMAPINFOHEADER
        {
            biSize = (uint)sizeof(BITMAPINFOHEADER),
            biWidth = size,
            biHeight = -size,
            biPlanes = 1,
            biBitCount = 32,
            biCompression = BI.BI_RGB,
        },
    };

    /// <summary>A 32-bit icon from BGRA pixels; its alpha is the shape.</summary>
    private static nint ToIcon(byte[] pixels, int size)
    {
        var screen = GetDC(HWND.NULL);
        HBITMAP color = HBITMAP.NULL, mask = HBITMAP.NULL;
        try
        {
            var header = Header(size);
            void* bits;
            color = CreateDIBSection(screen, &header, DIB_RGB_COLORS, &bits, HANDLE.NULL, 0);
            if (color == HBITMAP.NULL)
            {
                throw new InkRendererException($"couldn't draw the icon (error {GetLastError()})");
            }
            pixels.CopyTo(new Span<byte>(bits, pixels.Length));
            // The alpha channel is the shape; a monochrome mask of zeros leaves it to the alpha.
            var zeros = stackalloc byte[((size + 15) / 16) * 2 * size];
            mask = CreateBitmap(size, size, 1, 1, zeros);
            var made = new ICONINFO { fIcon = true, hbmColor = color, hbmMask = mask };
            var icon = CreateIconIndirect(&made);
            if (icon == HICON.NULL)
            {
                throw new InkRendererException($"couldn't make the icon (error {GetLastError()})");
            }
            return (nint)icon.Value;
        }
        finally
        {
            if (color != HBITMAP.NULL)
            {
                DeleteObject((HGDIOBJ)color.Value);
            }
            if (mask != HBITMAP.NULL)
            {
                DeleteObject((HGDIOBJ)mask.Value);
            }
            _ = ReleaseDC(HWND.NULL, screen);
        }
    }
}
