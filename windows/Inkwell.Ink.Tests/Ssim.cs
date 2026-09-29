// Structural similarity, single scale, as the Mac's ReferenceTests compute it (Wang, Bovik, Sheikh
// and Simoncelli 2004, and their reference implementation): Rec. 601 luminance, an 11 x 11 Gaussian
// window with sigma 1.5, K1 = 0.01, K2 = 0.03, statistics over the valid region only, the mean of
// the map. SsimTests checks the scorer on known answers before any score is trusted.
using Inkwell.Ink;

namespace Inkwell.Ink.Tests;

internal static class Ssim
{
    public const int Window = 11;
    private const double Sigma = 1.5;

    internal sealed record Map(double[] Values, int Width, int Height)
    {
        public double Mean => Values.Average();
    }

    public static Map Compute(double[] x, double[] y, int width, int height)
    {
        const double c1 = 0.01 * 255 * (0.01 * 255), c2 = 0.03 * 255 * (0.03 * 255);
        var g = Gaussian();
        var mu1 = Filter(x, width, height, g);
        var mu2 = Filter(y, width, height, g);
        var xx = Filter(x.Select(v => v * v).ToArray(), width, height, g);
        var yy = Filter(y.Select(v => v * v).ToArray(), width, height, g);
        var xy = Filter(x.Zip(y, (a, b) => a * b).ToArray(), width, height, g);
        var values = new double[mu1.Length];
        for (var i = 0; i < values.Length; i++)
        {
            double m1 = mu1[i], m2 = mu2[i];
            double s11 = xx[i] - m1 * m1, s22 = yy[i] - m2 * m2, s12 = xy[i] - m1 * m2;
            values[i] = (2 * m1 * m2 + c1) * (2 * s12 + c2) / ((m1 * m1 + m2 * m2 + c1) * (s11 + s22 + c2));
        }
        return new Map(values, width - Window + 1, height - Window + 1);
    }

    private static double[] Gaussian()
    {
        var half = (Window - 1) / 2.0;
        var g = Enumerable.Range(0, Window).Select(i => Math.Exp(-Math.Pow(i - half, 2) / (2 * Sigma * Sigma))).ToArray();
        var total = g.Sum();
        return g.Select(v => v / total).ToArray();
    }

    /// <summary>'valid' correlation with outer(g, g), separably.</summary>
    private static double[] Filter(double[] image, int w, int h, double[] g)
    {
        int k = g.Length, ow = w - k + 1, oh = h - k + 1;
        var rows = new double[oh * w];
        for (var y = 0; y < oh; y++)
        {
            for (var x = 0; x < w; x++)
            {
                var s = 0.0;
                for (var i = 0; i < k; i++)
                {
                    s += g[i] * image[(y + i) * w + x];
                }
                rows[y * w + x] = s;
            }
        }
        var output = new double[oh * ow];
        for (var y = 0; y < oh; y++)
        {
            for (var x = 0; x < ow; x++)
            {
                var s = 0.0;
                for (var j = 0; j < k; j++)
                {
                    s += g[j] * rows[y * w + x + j];
                }
                output[y * ow + x] = s;
            }
        }
        return output;
    }
}

/// <summary>The S0.5 criteria for one frame against a reference, as the Mac's Comparison.</summary>
internal sealed class Comparison
{
    public double WholeSsim { get; }
    /// <summary>Over pixels darker than luminance 180 in either frame, the letters included.</summary>
    public double InkSsim { get; }
    public int InkPixels { get; }
    /// <summary>The same outside the wordmark's box: the ink alone.</summary>
    public double InkSsimOutsideBox { get; }
    /// <summary>Pixels where any channel differs by more than 8/255, outside the box.</summary>
    public int OffOutsideBox { get; }
    public int MaxOutsideBox { get; }

    public Comparison(InkImage ours, RgbImage reference, InkRect box)
    {
        int w = ours.Width, h = ours.Height;
        var a = new double[w * h];
        var b = new double[w * h];
        int off = 0, maxOff = 0;
        for (var y = 0; y < h; y++)
        {
            for (var x = 0; x < w; x++)
            {
                a[y * w + x] = ours.Luminance(x, y);
                b[y * w + x] = reference.Luminance(x, y);
                if (box.Contains(x + 0.5, y + 0.5))
                {
                    continue;
                }
                var (r, g, bl) = ours.Pixel(x, y);
                var i = (y * w + x) * 3;
                var d = Math.Max(Math.Abs(r - reference.Rgb[i]), Math.Max(Math.Abs(g - reference.Rgb[i + 1]), Math.Abs(bl - reference.Rgb[i + 2])));
                maxOff = Math.Max(maxOff, d);
                if (d > 8)
                {
                    off++;
                }
            }
        }
        var map = Ssim.Compute(a, b, w, h);
        WholeSsim = map.Mean;
        double sum = 0, sumOutside = 0;
        int count = 0, countOutside = 0;
        const int r0 = Ssim.Window / 2;
        for (var y = 0; y < map.Height; y++)
        {
            for (var x = 0; x < map.Width; x++)
            {
                if (Math.Min(a[(y + r0) * w + x + r0], b[(y + r0) * w + x + r0]) >= 180)
                {
                    continue;
                }
                var v = map.Values[y * map.Width + x];
                sum += v;
                count++;
                if (!box.Contains(x + r0 + 0.5, y + r0 + 0.5))
                {
                    sumOutside += v;
                    countOutside++;
                }
            }
        }
        InkSsim = count > 0 ? sum / count : 1;
        InkPixels = count;
        InkSsimOutsideBox = countOutside > 0 ? sumOutside / countOutside : 1;
        OffOutsideBox = off;
        MaxOutsideBox = maxOff;
    }

    public override string ToString() =>
        FormattableString.Invariant(
            $"ssim={WholeSsim:F6} ssim_ink={InkSsim:F6} (ink_px={InkPixels}) ssim_ink_outside_box={InkSsimOutsideBox:F6} off>8_outside_box={OffOutsideBox} max_diff_outside_box={MaxOutsideBox}");
}
