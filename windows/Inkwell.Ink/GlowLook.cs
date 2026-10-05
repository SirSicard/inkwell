// The colours the ink and the Drop paint with, as the shell resolves them from the appearance
// settings and the mode's tokens (Inkwell.Core's Glow): the orb's two shades of each voice, the
// idle orb and the blotted ink; and the Drop's pill, which follows the mode. Each channel 0..1.
// Until the shell gives its own, the day mode's defaults (Indigo & Coral). At rest the orb leans
// from idle toward the dots by RestTint (0..1): its first shade toward yours, its second toward
// theirs; 0 rests in idle alone (the Mac's OrbPalette.restTint).
namespace Inkwell.Ink;

/// <summary>The orb's colours.</summary>
public sealed record GlowLook(
    bool Dark,
    (float R, float G, float B) YouA,
    (float R, float G, float B) YouB,
    (float R, float G, float B) ThemA,
    (float R, float G, float B) ThemB,
    (float R, float G, float B) Idle,
    (float R, float G, float B) Ink,
    float RestTint = 0)
{
    /// <summary>
    /// How far the orb at rest leans toward the dots, so each preset clearly shows at rest: the
    /// Mac's GlowColours.restTint, chosen with the main window's 0.7 behind text so that text keeps
    /// 4.5:1 and secondary text 3:1 over it with every preset in both modes.
    /// </summary>
    public const float ShellRestTint = 0.6f;

    /// <summary>Day, Indigo &amp; Coral.</summary>
    public static GlowLook Default { get; } = new(
        false,
        Rgb.Of(0x6B5CFF), Rgb.Lift(Rgb.Of(0x6B5CFF), 0.4f),
        Rgb.Of(0xFFA34D), Rgb.Lift(Rgb.Of(0xFFA34D), 0.4f),
        Rgb.Of(0xC7BFDB), Rgb.Of(0x1D1B2E));
}

/// <summary>The Drop's pill: the mode's background, border, text and button colours.</summary>
public sealed record DropLook(
    bool Dark,
    (float R, float G, float B) Background,
    (float R, float G, float B) Border,
    (float R, float G, float B) Text,
    (float R, float G, float B) Secondary,
    (float R, float G, float B) Alert,
    (float R, float G, float B) ButtonFill,
    (float R, float G, float B) ButtonLabel)
{
    /// <summary>Day.</summary>
    public static DropLook Default { get; } = new(
        false, Rgb.Of(0xFBF8F4), Rgb.Of(0xE2DACE), Rgb.Of(0x1D1B2E), Rgb.Of(0x6A6577), Rgb.Of(0xB23A26),
        Rgb.Of(0x1D1B2E), Rgb.Of(0xFBF8F4));
}

/// <summary>Colour helpers.</summary>
public static class Rgb
{
    /// <summary>From 0xRRGGBB.</summary>
    public static (float R, float G, float B) Of(int hex) =>
        (((hex >> 16) & 0xFF) / 255f, ((hex >> 8) & 0xFF) / 255f, (hex & 0xFF) / 255f);

    /// <summary>Towards white: c + (1 - c) * k.</summary>
    public static (float R, float G, float B) Lift((float R, float G, float B) c, float k) =>
        (c.R + (1 - c.R) * k, c.G + (1 - c.G) * k, c.B + (1 - c.B) * k);

    /// <summary>a + (b - a) * k.</summary>
    public static (float R, float G, float B) Mix((float R, float G, float B) a, (float R, float G, float B) b, float k) =>
        (a.R + (b.R - a.R) * k, a.G + (b.G - a.G) * k, a.B + (b.B - a.B) * k);
}
