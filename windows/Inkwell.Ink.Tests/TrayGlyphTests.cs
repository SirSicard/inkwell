// The live icon's pictures, drawn from the app's real icon (Assets/Inkwell.ico) at the sizes the
// tray uses at 100 %, 150 % and 200 % (16, 24 and 32 pixels): at rest the icon as it is, a dot in
// the orb's place in the state's colour (dimmer as the pulse breathes out), and the rim filling
// clockwise from the top with the final pass. Each is also written out as a PNG for a look
// (INK_GLYPH_OUT, when set), as the Mac's RenderTests write the menu bar's.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class TrayGlyphTests
{
    private static readonly string Icon = Path.Combine(AppContext.BaseDirectory, "Inkwell.ico");
    private static readonly (float R, float G, float B) Them = Rgb.Of(0xFFA34D);
    private static readonly (float R, float G, float B) You = Rgb.Of(0x6B5CFF);

    private static (byte R, byte G, byte B, byte A) At(byte[] bgra, int size, double x, double y)
    {
        var i = ((int)(y * size) * size + (int)(x * size)) * 4;
        return (bgra[i + 2], bgra[i + 1], bgra[i], bgra[i + 3]);
    }

    private static void Save(byte[] bgra, int size, string name)
    {
        if (Environment.GetEnvironmentVariable("INK_GLYPH_OUT") is not { Length: > 0 } dir)
        {
            return;
        }
        Directory.CreateDirectory(dir);
        // A 24-bit BMP over a mid grey, enlarged 8 times, so it shows as a tray would show it.
        const int k = 8;
        var w = size * k;
        var row = (w * 3 + 3) & ~3;
        var data = new byte[54 + row * w];
        BitConverter.GetBytes((ushort)0x4D42).CopyTo(data, 0);
        BitConverter.GetBytes(data.Length).CopyTo(data, 2);
        BitConverter.GetBytes(54).CopyTo(data, 10);
        BitConverter.GetBytes(40).CopyTo(data, 14);
        BitConverter.GetBytes(w).CopyTo(data, 18);
        BitConverter.GetBytes(w).CopyTo(data, 22);
        BitConverter.GetBytes((ushort)1).CopyTo(data, 26);
        BitConverter.GetBytes((ushort)24).CopyTo(data, 28);
        for (var y = 0; y < w; y++)
        {
            for (var x = 0; x < w; x++)
            {
                var i = ((y / k) * size + x / k) * 4;
                var a = bgra[i + 3] / 255.0;
                var o = 54 + (w - 1 - y) * row + x * 3;
                for (var c = 0; c < 3; c++)
                {
                    data[o + c] = (byte)Math.Round(bgra[i + c] * a + 0x30 * (1 - a));
                }
            }
        }
        File.WriteAllBytes(Path.Combine(dir, $"{name}-{size}.bmp"), data);
    }

    [Theory]
    [InlineData(16)]
    [InlineData(24)]
    [InlineData(32)]
    public void ADotTakesTheOrbsPlaceInTheStatesColour(int size)
    {
        Assert.True(File.Exists(Icon), Icon);
        var rest = TrayGlyph.Load(Icon, size);
        Save(rest, size, "rest");
        var dot = TrayGlyph.Load(Icon, size);
        TrayGlyph.Paint(dot, size, GlyphLook.Dot, Them, 1, null);
        Save(dot, size, "recording");
        // The centre is their colour, opaque; the plate's corner and the rim are the icon's own.
        var centre = At(dot, size, TrayGlyph.OrbX, TrayGlyph.OrbY);
        Assert.Equal(((byte)0xFF, (byte)0xA3, (byte)0x4D, (byte)255), centre);
        Assert.Equal(At(rest, size, 0.08, 0.5), At(dot, size, 0.08, 0.5));
        Assert.Equal(At(rest, size, 0, 0), At(dot, size, 0, 0));

        // Breathed out: lighter, toward the colour's tint, never darker (coral went brown over night).
        var faint = TrayGlyph.Load(Icon, size);
        TrayGlyph.Paint(faint, size, GlyphLook.Dot, Them, 0.55, null);
        Save(faint, size, "recording-breath-out");
        var out_ = At(faint, size, TrayGlyph.OrbX, TrayGlyph.OrbY);
        Assert.True(out_.R >= centre.R && out_.G > centre.G && out_.B > centre.B, $"{out_}");

        var dictating = TrayGlyph.Load(Icon, size);
        TrayGlyph.Paint(dictating, size, GlyphLook.Dot, You, 1, null);
        Save(dictating, size, "dictating");
        Assert.Equal(((byte)0x6B, (byte)0x5C, (byte)0xFF, (byte)255), At(dictating, size, TrayGlyph.OrbX, TrayGlyph.OrbY));
        var problem = TrayGlyph.Load(Icon, size);
        TrayGlyph.Paint(problem, size, GlyphLook.Dot, Rgb.Of(0xE5484D), 1, null);
        Save(problem, size, "problem");
    }

    /// <summary>The rim fills clockwise from the top: at a quarter the top right is lit and the left is not; with no number it is dashed.</summary>
    [Theory]
    [InlineData(16)]
    [InlineData(24)]
    [InlineData(32)]
    public void TheRimFillsClockwiseFromTheTop(int size)
    {
        int Lit(byte[] bgra) => Enumerable.Range(0, size * size).Count(p => bgra[p * 4 + 2] > 0xE0 && bgra[p * 4 + 1] > 0x90 && bgra[p * 4 + 1] < 0xB8 && bgra[p * 4] < 0x80);
        var counts = new List<int>();
        foreach (var progress in new double?[] { 0.25, 0.5, 1, null })
        {
            var ring = TrayGlyph.Load(Icon, size);
            TrayGlyph.Paint(ring, size, GlyphLook.Ring, Them, 1, progress);
            Save(ring, size, $"finalpass-{(progress is double p ? (int)(p * 100) : "dashed")}");
            counts.Add(Lit(ring));
        }
        Assert.True(counts[0] > 0 && counts[0] < counts[1] && counts[1] < counts[2], string.Join(",", counts));
        Assert.True(counts[3] > counts[0] && counts[3] < counts[2], $"dashed: about half the rim, {counts[3]}");
    }
}
