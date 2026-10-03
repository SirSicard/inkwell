// The window's orb's opacity over its backdrop, faded toward a target over 0.8 s: dimmed to 30%
// while anything is live so the text over it reads, whole again at rest (InkPanel). A frame reads
// the value at its own time; nothing ticks for the fade itself. Without animations (Windows'
// Animation effects off, or Always still) it jumps.
namespace Inkwell.Ink;

/// <summary>An opacity fading linearly toward a target. Times are in seconds, on any one clock.</summary>
public sealed class OrbFade
{
    /// <summary>How long a fade takes.</summary>
    public const double Duration = 0.8;

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
