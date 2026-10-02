// How Glow's you and them colours are resolved for a mode, the same on both shells, from the
// generated tokens (GlowTokens, design/tokens.json):
//
//   1. take the mode's preset (you and them);
//   2. a custom #rrggbb replaces the preset's colour;
//   3. fit it to the mode: at night a colour too dark is lifted towards white, by day one too pale
//      is dimmed (GlowTokens.Fit);
//   4. its partner, the orb's second shade, is lifted towards white (Fit.PartnerLift);
//   5. the idle orb and the ink are the mode's tokens.
//
// The you and them colours paint the orb, the edge glow, the Drop's orb, the transcript's speaker
// dots, the waveform's lanes, the Live legend and the tray icon's state dot. Text never uses them.
//
// The edge glow's gradient is here too (GlowEdgeGradient): one horizontal gradient round the
// whole edge, yours on the left, theirs on the right, blending in a band that leans away from
// whoever is speaking; your colour throughout while you dictate; out once the call blots.
using System.Globalization;

namespace Inkwell.Core.Glow;

/// <summary>An sRGB colour, each channel 0..1.</summary>
public readonly record struct GlowRgb(double R, double G, double B)
{
    /// <summary>A token's colour (its opacity is the caller's).</summary>
    public static GlowRgb From(GlowColor c) => new(c.Red, c.Green, c.Blue);

    /// <summary>From 0xRRGGBB.</summary>
    public static GlowRgb FromInt(int rgb) => new(((rgb >> 16) & 0xFF) / 255.0, ((rgb >> 8) & 0xFF) / 255.0, (rgb & 0xFF) / 255.0);

    /// <summary>"#rrggbb" (either case), or null for anything else.</summary>
    public static GlowRgb? Parse(string? hex)
    {
        if (hex is not { Length: 7 } || hex[0] != '#'
            || !int.TryParse(hex.AsSpan(1), NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out var rgb))
        {
            return null;
        }
        return FromInt(rgb);
    }

    /// <summary>"#rrggbb", lowercase, as the settings store it.</summary>
    public string Hex => string.Create(CultureInfo.InvariantCulture, $"#{Byte(R):x2}{Byte(G):x2}{Byte(B):x2}");

    /// <summary>Each channel as a byte (0..255).</summary>
    public (byte R, byte G, byte B) Bytes => (Byte(R), Byte(G), Byte(B));

    /// <summary>The tokens' luminance: LumaRed r + LumaGreen g + LumaBlue b.</summary>
    public double Luminance => GlowTokens.Fit.LumaRed * R + GlowTokens.Fit.LumaGreen * G + GlowTokens.Fit.LumaBlue * B;

    /// <summary>Towards white: c + (1 - c) * k.</summary>
    public GlowRgb Lift(double k) => new(R + (1 - R) * k, G + (1 - G) * k, B + (1 - B) * k);

    /// <summary>Towards black: c * k.</summary>
    public GlowRgb Scale(double k) => new(R * k, G * k, B * k);

    /// <summary>a + (b - a) * k, per channel.</summary>
    public static GlowRgb Mix(GlowRgb a, GlowRgb b, double k) =>
        new(a.R + (b.R - a.R) * k, a.G + (b.G - a.G) * k, a.B + (b.B - a.B) * k);

    private static byte Byte(double v) => (byte)Math.Round(Math.Clamp(v, 0, 1) * 255, MidpointRounding.AwayFromZero);
}

/// <summary>The colours a mode resolves to: the orb's two shades of each voice, the idle orb and the ink.</summary>
public sealed record GlowColours(bool Dark, GlowRgb You, GlowRgb YouPartner, GlowRgb Them, GlowRgb ThemPartner, GlowRgb Idle, GlowRgb Ink);

/// <summary>The presets and the colour rule.</summary>
public static class GlowScheme
{
    /// <summary>The preset a mode starts with, and the one an unknown id falls back to (C1's default).</summary>
    public const string DefaultPreset = "indigo";

    /// <summary>A mode's tokens.</summary>
    public static GlowPalette Palette(bool dark) => dark ? GlowTokens.Dark : GlowTokens.Light;

    /// <summary>The presets, in the order Settings shows them.</summary>
    public static IReadOnlyList<GlowPreset> Presets => GlowTokens.Presets;

    /// <summary>The preset with this id, or the default.</summary>
    public static GlowPreset Preset(string? id) =>
        Presets.FirstOrDefault(p => p.Id == id) ?? Presets.First(p => p.Id == DefaultPreset);

    /// <summary>Whether a preset has this id.</summary>
    public static bool IsPreset(string? id) => Presets.Any(p => p.Id == id);

    /// <summary>Step 3: a colour too dark for night is lifted, one too pale for day dimmed, so it stays visible.</summary>
    public static GlowRgb Fit(GlowRgb c, bool dark) =>
        dark && c.Luminance < GlowTokens.Fit.DarkLiftBelow ? c.Lift(GlowTokens.Fit.DarkLift)
        : !dark && c.Luminance > GlowTokens.Fit.LightDimAbove ? c.Scale(GlowTokens.Fit.LightDim)
        : c;

    /// <summary>Step 4: the orb's second shade of a colour.</summary>
    public static GlowRgb Partner(GlowRgb c) => c.Lift(GlowTokens.Fit.PartnerLift);

    /// <summary>The colours of a mode: its preset, the user's own colours over it, fitted to the mode.</summary>
    public static GlowColours Resolve(bool dark, string? preset, GlowRgb? you = null, GlowRgb? them = null)
    {
        var p = Preset(preset);
        var y = Fit(you ?? GlowRgb.From(p.You), dark);
        var t = Fit(them ?? GlowRgb.From(p.Them), dark);
        var tokens = Palette(dark);
        return new GlowColours(dark, y, Partner(y), t, Partner(t), GlowRgb.From(tokens.IdleOrb), GlowRgb.From(tokens.Ink));
    }
}

/// <summary>A stop of the edge glow's gradient: where along the width (0..1), its colour and its opacity.</summary>
public readonly record struct GlowEdgeStop(double Offset, GlowRgb Colour, double Alpha);

/// <summary>The edge glow's gradient, as the canvas computes it.</summary>
public static class GlowEdgeGradient
{
    /// <summary>
    /// The five stops for the state weights (dictating <paramref name="d"/>, meeting
    /// <paramref name="m"/>, blotting <paramref name="b"/>), your level and theirs, and the time in
    /// seconds (a still frame passes 0), into <paramref name="stops"/>; false when nothing would show.
    /// </summary>
    public static bool Stops(double d, double m, double b, double you, double them, double time, GlowColours colours, Span<GlowEdgeStop> stops)
    {
        ArgumentNullException.ThrowIfNull(colours);
        if (stops.Length < 5)
        {
            throw new ArgumentException("the gradient has five stops", nameof(stops));
        }
        var a1 = d * (0.3 + 0.7 * you) + m * (1 - b) * (0.25 + 0.75 * you);
        var a2 = m * (1 - b) * (0.25 + 0.75 * them);
        var aR = a1 + (a2 - a1) * m;
        if (a1 < 0.01 && aR < 0.01)
        {
            return false;
        }
        var c = Math.Clamp(0.5 + GlowTokens.EdgeGlow.Lean * (you - them) * m, 0.28, 0.72);
        var flow = Math.Sin(time * GlowTokens.EdgeGlow.FlowRate) * GlowTokens.EdgeGlow.FlowAmplitude;
        var hue = 0.5 + 0.5 * Math.Sin(time * 0.9);
        var band = GlowTokens.EdgeGlow.BlendBand;
        GlowRgb yA = colours.You, yB = colours.YouPartner, tA = colours.Them, tB = colours.ThemPartner;
        stops[0] = new(0, GlowRgb.Mix(yA, yB, hue), a1);
        stops[1] = new(Offset(c - band + flow), GlowRgb.Mix(yB, yA, hue), a1);
        stops[2] = new(Offset(c), GlowRgb.Mix(yB, tA, 0.5 * m), (a1 + aR) / 2);
        stops[3] = new(Offset(c + band + flow), GlowRgb.Mix(yA, tA, m), aR);
        stops[4] = new(1, GlowRgb.Mix(yB, tB, m), aR);
        return true;
    }

    private static double Offset(double x) => Math.Clamp(x, 0, 1);
}
