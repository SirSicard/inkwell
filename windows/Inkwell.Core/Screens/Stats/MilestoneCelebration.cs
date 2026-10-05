// A milestone reached, as the Mac's MilestoneCelebration: a short, quiet glow over the orb in your two
// colours, and one line at the foot of the window, once. Only while the window is on screen
// (nothing moves unseen): a milestone reached while the user dictates in another app waits for
// them. Under "Always still" or with Windows' animation effects off there is no glow, only the
// line. Narrator hears the line.
namespace Inkwell.Core.Screens;

public static class MilestoneCelebration
{
    /// <summary>How long the line shows.</summary>
    public static readonly TimeSpan Shown = TimeSpan.FromSeconds(6);

    /// <summary>The glow: in, held, out. About two and a half seconds, once.</summary>
    public static readonly TimeSpan GlowIn = TimeSpan.FromSeconds(0.8);
    public static readonly TimeSpan GlowHeld = TimeSpan.FromSeconds(0.6);
    public static readonly TimeSpan GlowOut = TimeSpan.FromSeconds(1.2);

    /// <summary>The glow's brightest, over the orb: quiet, under the screens' text.</summary>
    public const double GlowPeak = 0.45;

    /// <summary>The glow's size: this many of the orb's units across (the Mac's 1.1).</summary>
    public const double GlowUnits = 1.1;

    /// <summary>What Narrator calls the line's dismiss button.</summary>
    public const string DismissName = "Dismiss";

    /// <summary>The celebration to show now: the pending one, while the window is on screen.</summary>
    public static StatsModel.Celebration? Showing(StatsModel.Celebration? pending, bool onScreen) => onScreen ? pending : null;

    /// <summary>Whether the glow moves at all: never under "Always still" or with animations off.</summary>
    public static bool Glows(bool still, bool reduceMotion) => !still && !reduceMotion;
}
