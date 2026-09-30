// Just enough PNG for the tests: read the Mac's Metal renders (8-bit RGB or RGBA, not interlaced)
// and write this renderer's frames for a person to look at. No colour conversion either way: the
// stored bytes, as the Mac's reference comparison reads them.
using System.Buffers.Binary;
using System.IO.Compression;

namespace Inkwell.Ink.Tests;

internal sealed class RgbImage(int width, int height, byte[] rgb)
{
    public int Width { get; } = width;
    public int Height { get; } = height;
    /// <summary>3 bytes per pixel, row 0 at the top.</summary>
    public byte[] Rgb { get; } = rgb;

    public double Luminance(int x, int y)
    {
        var i = (y * Width + x) * 3;
        return 0.299 * Rgb[i] + 0.587 * Rgb[i + 1] + 0.114 * Rgb[i + 2];
    }
}

internal static class Png
{
    private static readonly byte[] Signature = [137, 80, 78, 71, 13, 10, 26, 10];

    public static RgbImage Read(string path)
    {
        var file = File.ReadAllBytes(path);
        if (!file.AsSpan(0, 8).SequenceEqual(Signature))
        {
            throw new InvalidDataException($"{Path.GetFileName(path)} is not a PNG");
        }
        int width = 0, height = 0, channels = 0;
        using var idat = new MemoryStream();
        for (var at = 8; at < file.Length;)
        {
            var length = BinaryPrimitives.ReadInt32BigEndian(file.AsSpan(at));
            var type = System.Text.Encoding.ASCII.GetString(file, at + 4, 4);
            var data = file.AsSpan(at + 8, length);
            if (type == "IHDR")
            {
                width = BinaryPrimitives.ReadInt32BigEndian(data);
                height = BinaryPrimitives.ReadInt32BigEndian(data[4..]);
                var (depth, colour, interlace) = (data[8], data[9], data[12]);
                channels = colour switch { 2 => 3, 6 => 4, _ => 0 };
                if (depth != 8 || channels == 0 || interlace != 0)
                {
                    throw new InvalidDataException($"{Path.GetFileName(path)}: only 8-bit RGB or RGBA, not interlaced");
                }
            }
            else if (type == "IDAT")
            {
                idat.Write(data);
            }
            else if (type == "IEND")
            {
                break;
            }
            at += 12 + length;
        }
        idat.Position = 0;
        using var z = new ZLibStream(idat, CompressionMode.Decompress);
        var stride = width * channels;
        var raw = new byte[(stride + 1) * height];
        z.ReadExactly(raw);
        var rgb = new byte[width * height * 3];
        var prev = new byte[stride];
        var cur = new byte[stride];
        for (var y = 0; y < height; y++)
        {
            var filter = raw[y * (stride + 1)];
            raw.AsSpan(y * (stride + 1) + 1, stride).CopyTo(cur);
            for (var i = 0; i < stride; i++)
            {
                int a = i >= channels ? cur[i - channels] : 0, b = prev[i], c = i >= channels ? prev[i - channels] : 0;
                cur[i] = (byte)(cur[i] + filter switch
                {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => (a + b) / 2,
                    4 => Paeth(a, b, c),
                    _ => throw new InvalidDataException($"PNG filter {filter}"),
                });
            }
            for (var x = 0; x < width; x++)
            {
                cur.AsSpan(x * channels, 3).CopyTo(rgb.AsSpan((y * width + x) * 3));
            }
            (prev, cur) = (cur, prev);
        }
        return new RgbImage(width, height, rgb);
    }

    private static int Paeth(int a, int b, int c)
    {
        int p = a + b - c, pa = Math.Abs(p - a), pb = Math.Abs(p - b), pc = Math.Abs(p - c);
        return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
    }

    /// <summary>Writes 8-bit RGBA, row 0 at the top.</summary>
    public static void Write(string path, int width, int height, byte[] rgba)
    {
        using var file = File.Create(path);
        file.Write(Signature);
        var header = new byte[13];
        BinaryPrimitives.WriteInt32BigEndian(header, width);
        BinaryPrimitives.WriteInt32BigEndian(header.AsSpan(4), height);
        header[8] = 8;
        header[9] = 6;
        Chunk(file, "IHDR", header);
        using var packed = new MemoryStream();
        using (var z = new ZLibStream(packed, CompressionLevel.Optimal, leaveOpen: true))
        {
            for (var y = 0; y < height; y++)
            {
                z.WriteByte(0);
                z.Write(rgba, y * width * 4, width * 4);
            }
        }
        Chunk(file, "IDAT", packed.ToArray());
        Chunk(file, "IEND", []);
    }

    private static void Chunk(Stream s, string type, byte[] data)
    {
        var length = new byte[4];
        BinaryPrimitives.WriteInt32BigEndian(length, data.Length);
        s.Write(length);
        var body = new byte[4 + data.Length];
        System.Text.Encoding.ASCII.GetBytes(type, body);
        data.CopyTo(body, 4);
        s.Write(body);
        var crc = new byte[4];
        BinaryPrimitives.WriteUInt32BigEndian(crc, Crc32(body));
        s.Write(crc);
    }

    private static uint Crc32(byte[] data)
    {
        var crc = 0xFFFFFFFFu;
        foreach (var b in data)
        {
            crc ^= b;
            for (var k = 0; k < 8; k++)
            {
                crc = (crc & 1) != 0 ? 0xEDB88320u ^ (crc >> 1) : crc >> 1;
            }
        }
        return ~crc;
    }
}
