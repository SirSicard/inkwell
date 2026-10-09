// The window's orb's opacity over its backdrop, faded toward a target over 0.8 s (InkPanel): behind
// the main window's text at 70% at rest and 30% while anything is live, so the text over it reads
// (BehindText). A frame reads the value at its own time; nothing ticks for the fade itself. Without
// animations (Windows' Animation effects off, or Always still) it jumps.
namespace Inkwell.Ink;

/// <summary>An opacity fading linearly toward a target. Times are in seconds, on any one clock.</summary>
public sealed class OrbFade
{
    /// <summary>How long a fade takes.</summary>
    public const double Duration = 0.8;

    /// <summary>
    /// The orb behind the main window's text, as the Mac's OrbLayer: live at 30 %, where its bright
    /// centre still leaves the mode's text at 4.5:1 and secondary text at 3:1; at rest, leaning
    /// toward the preset (GlowLook.ShellRestTint), at 70 %, chosen with that tint for the same
    /// contrast; under High Contrast never above 45 %.
    /// </summary>
    public const float LiveBehindText = 0.3f;
    public const float RestBehindText = 0.7f;
    public const float Dimmed = 0.45f;

    /// <summary>The live opacity at the strongest setting (1): the orb still steps back, but stays plain to see.</summary>
    public const float StrongestLive = 0.8f;

    /// <summary>
    /// The main window's orb's opacity: <paramref name="live"/> while anything is live,
    /// <paramref name="dimmed"/> under High Contrast. <paramref name="rest"/> is the user's strength
    /// (Settings > Appearance, 0.1 to 1): the opacity at rest. Live, up to the default it keeps the
    /// default's ratio (30 % to 70 %); above it, live climbs to <see cref="StrongestLive"/> at 1, so
    /// a stronger setting shows during a call too (at 3/7 it stayed faint). Only the default (0.7)
    /// is the strength every text was checked against.
    /// </summary>
    public static float BehindText(bool live, bool dimmed, float rest = RestBehindText)
    {
        var atRest = Math.Clamp(rest, 0.1f, 1f);
        var whileLive = atRest <= RestBehindText
            ? atRest * (LiveBehindText / RestBehindText)
            : LiveBehindText + ((atRest - RestBehindText) / (1 - RestBehindText) * (StrongestLive - LiveBehindText));
        return Math.Min(dimmed ? Dimmed : 1, live ? whileLive : atRest);
    }

    private float from = 1;
    private double start;

    /// <summary>Where it is going.</summary>
    public float Target { get; private set; } = 1;

    /// <summary>Fades from where it is at <paramref name="now"/> to <paramref name="target"/>, or jumps there when not <paramref name="animate"/>.</summary>
    public void FadeTo(float target, double now, bool animate)
    {
        from = animate ? ValueAt(now) : target;
        Target = target;
        start = now;
    }

    /// <summary>The opacity at <paramref name="now"/>.</summary>
    public float ValueAt(double now)
    {
        var t = Math.Clamp((now - start) / Duration, 0, 1);
        return (float)(from + ((Target - from) * t));
    }

    /// <summary>Whether it is still on its way at <paramref name="now"/>.</summary>
    public bool Fading(double now) => ValueAt(now) != Target;
}
