// The tray icon's four states, drawn in code from the app's own icon (Assets/Inkwell.ico): idle is
// the icon as it is; dictating, recording and a problem add a round dot in the lower right, in your
// colour, theirs, or the alert colour, cut out of the icon by a clear ring so it reads on any
// taskbar. No icon files are added: the colours are the user's, so the dots are drawn when the
// colours or the state change (never on a timer). GDI only, at the small-icon size.
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink;

public static unsafe class TrayGlyph
{
    /// <summary>
    /// The icon at <paramref name="path"/> at the small-icon size, with a dot of
    /// <paramref name="dot"/> (none: the plain icon). The caller owns the HICON (DestroyIcon).
    /// Throws <see cref="InkRendererException"/> when the icon cannot be read or made.
    /// </summary>
    public static nint Make(string path, (float R, float G, float B)? dot)
    {
        ArgumentNullException.ThrowIfNull(path);
        var size = GetSystemMetrics(SM.SM_CXSMICON);
        HICON source;
        fixed (char* file = path)
        {
            source = (HICON)LoadImageW(HINSTANCE.NULL, file, IMAGE.IMAGE_ICON, size, size, LR.LR_LOADFROMFILE);
        }
        if (source == HICON.NULL)
        {
            throw new InkRendererException($"couldn't load the tray icon (error {GetLastError()})");
        }
        if (dot is not { } colour)
        {
            return (nint)source.Value;
        }
        try
        {
            return Dotted(source, size, colour);
        }
        finally
        {
            DestroyIcon(source);
        }
    }

    /// <summary>Releases an icon <see cref="Make"/> made.</summary>
    public static void Destroy(nint icon)
    {
        if (icon != 0)
        {
            DestroyIcon((HICON)icon);
        }
    }

    private static nint Dotted(HICON source, int size, (float R, float G, float B) colour)
    {
        ICONINFO info;
        if (!GetIconInfo(source, &info))
        {
            throw new InkRendererException($"couldn't read the tray icon (error {GetLastError()})");
        }
        var screen = GetDC(HWND.NULL);
        HBITMAP color = HBITMAP.NULL, mask = HBITMAP.NULL;
        try
        {
            var header = new BITMAPINFO
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
            var pixels = new byte[size * size * 4];
            fixed (byte* bgra = pixels)
            {
                if (info.hbmColor == HBITMAP.NULL || GetDIBits(screen, info.hbmColor, 0, (uint)size, bgra, &header, DIB_RGB_COLORS) == 0)
                {
                    throw new InkRendererException("couldn't read the tray icon's pixels");
                }
            }
            Dot(pixels, size, colour);
            void* bits;
            color = CreateDIBSection(screen, &header, DIB_RGB_COLORS, &bits, HANDLE.NULL, 0);
            if (color == HBITMAP.NULL)
            {
                throw new InkRendererException($"couldn't draw the tray icon (error {GetLastError()})");
            }
            pixels.CopyTo(new Span<byte>(bits, pixels.Length));
            // The alpha channel is the shape; a monochrome mask of zeros leaves it to the alpha.
            var zeros = stackalloc byte[((size + 15) / 16) * 2 * size];
            mask = CreateBitmap(size, size, 1, 1, zeros);
            var made = new ICONINFO { fIcon = true, hbmColor = color, hbmMask = mask };
            var icon = CreateIconIndirect(&made);
            if (icon == HICON.NULL)
            {
                throw new InkRendererException($"couldn't make the tray icon (error {GetLastError()})");
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

    /// <summary>
    /// A dot in the lower right of a BGRA icon (straight alpha, row 0 at the top): a clear ring
    /// first, then the dot, both antialiased by 4 x 4 samples per pixel.
    /// </summary>
    internal static void Dot(byte[] bgra, int size, (float R, float G, float B) colour)
    {
        var radius = size * 0.26;
        var ring = radius + Math.Max(1, size / 16.0);
        var cx = size - radius - 0.5;
        var cy = size - radius - 0.5;
        for (var y = 0; y < size; y++)
        {
            for (var x = 0; x < size; x++)
            {
                double inDot = 0, inRing = 0;
                for (var sy = 0; sy < 4; sy++)
                {
                    for (var sx = 0; sx < 4; sx++)
                    {
                        var d = Math.Sqrt(Math.Pow(x + (sx + 0.5) / 4 - 0.5 - cx, 2) + Math.Pow(y + (sy + 0.5) / 4 - 0.5 - cy, 2));
                        inDot += d <= radius ? 1 : 0;
                        inRing += d <= ring ? 1 : 0;
                    }
                }
                inDot /= 16;
                inRing /= 16;
                if (inRing == 0)
                {
                    continue;
                }
                var i = (y * size + x) * 4;
                // Clear the ring: the icon's own alpha fades out under it.
                var a = bgra[i + 3] / 255.0 * (1 - inRing);
                // The dot over what is left.
                var outA = inDot + a * (1 - inDot);
                if (outA <= 0)
                {
                    bgra[i] = bgra[i + 1] = bgra[i + 2] = bgra[i + 3] = 0;
                    continue;
                }
                bgra[i] = Channel((colour.B * inDot + bgra[i] / 255.0 * a * (1 - inDot)) / outA);
                bgra[i + 1] = Channel((colour.G * inDot + bgra[i + 1] / 255.0 * a * (1 - inDot)) / outA);
                bgra[i + 2] = Channel((colour.R * inDot + bgra[i + 2] / 255.0 * a * (1 - inDot)) / outA);
                bgra[i + 3] = Channel(outA);
            }
        }
    }

    private static byte Channel(double v) => (byte)Math.Round(Math.Clamp(v, 0, 1) * 255);
}
