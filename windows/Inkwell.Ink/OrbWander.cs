// Where the main window's orb goes, so it is not always in the same place, ported from the Mac
// (mac/Sources/InkRenderer/OrbWander.swift): a path of eased legs between random spots inside a
// region of the canvas. Live, one leg follows another, slowly, on the frames the orb already draws.
// At rest the orb glides to a new spot only when asked (the surface asks when it comes on screen,
// when the screen or monitor behind it changes, and when its window is activated after RestInterval
// in one spot) and then holds still, so a resting orb still draws nothing between moves
// (architecture rule 9).
//
// A value with its own random source: seeded, the same calls give the same path (the tests).
namespace Inkwell.Ink;

/// <summary>The orb's centre over time, as fractions of the canvas (x from the left, y from the top).</summary>
public struct OrbWander
{
    /// <summary>The region the centre stays in.</summary>
    public readonly record struct Bounds(double MinX, double MaxX, double MinY, double MaxY)
    {
        public bool Contains((double X, double Y) p) => p.X >= MinX && p.X <= MaxX && p.Y >= MinY && p.Y <= MaxY;

        /// <summary>The four corners: the region's extremes, which the contrast checks render.</summary>
        public IReadOnlyList<(double X, double Y)> Corners => [(MinX, MinY), (MaxX, MinY), (MinX, MaxY), (MaxX, MaxY)];

        /// <summary>The corner-to-corner distance, in the same fractions.</summary>
        public double Diagonal => Math.Sqrt((MaxX - MinX) * (MaxX - MinX) + (MaxY - MinY) * (MaxY - MinY));

        internal (double X, double Y) Clamped((double X, double Y) p) => (Math.Clamp(p.X, MinX, MaxX), Math.Clamp(p.Y, MinY, MaxY));
    }

    /// <summary>The main window's region, around its home (0.56, 0.26): clear of the navigation and mostly in the top half, behind the screens' headings (the Mac's Glow.Orb.wander).</summary>
    public static Bounds Main { get; } = new(0.46, 0.68, 0.18, 0.36);

    /// <summary>Seconds a glide at rest takes.</summary>
    public const double RestGlide = 2.4;

    /// <summary>Seconds a resting orb holds its spot before its window being activated again moves it.</summary>
    public const double RestInterval = 150;

    /// <summary>A new spot is at least this share of the region's diagonal away from the old one.</summary>
    public const double MinimumHop = 1.0 / 3;

    /// <summary>
    /// Live legs: their mean speed in fractions of the canvas per second (an eased leg peaks at
    /// 1.875 times it, about 0.022), and the shortest a leg takes.
    /// </summary>
    internal const double Cruise = 0.012;
    internal const double ShortestLeg = 12;

    /// <summary>One eased move: from, to, when it starts and how long it takes (0: it is there).</summary>
    private readonly record struct Leg((double X, double Y) From, (double X, double Y) To, double Start, double Duration);

    private InkRandom random;
    private Leg leg;

    /// <summary>An orb at <paramref name="start"/> (pulled into the bounds), still.</summary>
    public OrbWander(Bounds bounds, (double X, double Y) start, InkRandom random)
    {
        Region = bounds;
        this.random = random;
        var p = bounds.Clamped(start);
        leg = new Leg(p, p, 0, 0);
    }

    public Bounds Region { get; }

    /// <summary>Where the centre is at time <paramref name="t"/> (seconds, any clock the caller keeps to).</summary>
    public readonly (double X, double Y) Position(double t)
    {
        if (leg.Duration <= 0)
        {
            return leg.To;
        }
        var u = Math.Clamp((t - leg.Start) / leg.Duration, 0, 1);
        // Smootherstep: no speed and no acceleration at either end, so legs join without a kink.
        var e = u * u * u * (u * (u * 6 - 15) + 10);
        return (leg.From.X + (leg.To.X - leg.From.X) * e, leg.From.Y + (leg.To.Y - leg.From.Y) * e);
    }

    /// <summary>Whether it is on its way somewhere at <paramref name="t"/>.</summary>
    public readonly bool IsMoving(double t) => leg.Duration > 0 && t < leg.Start + leg.Duration;

    /// <summary>Live: once a leg is done, the next starts from there toward a new spot, slowly.</summary>
    public void Wander(double t)
    {
        if (IsMoving(t))
        {
            return;
        }
        var from = Position(t);
        var to = NextSpot(from);
        var distance = Distance(from, to);
        leg = new Leg(from, to, t, Math.Max(ShortestLeg, distance / Cruise));
    }

    /// <summary>At rest: to a new spot, gliding over RestGlide, or at once when not <paramref name="animated"/>.</summary>
    public void Move(double t, bool animated)
    {
        var from = Position(t);
        var to = NextSpot(from);
        leg = new Leg(from, to, t, animated ? RestGlide : 0);
    }

    /// <summary>Stops where it is at <paramref name="t"/>.</summary>
    public void Hold(double t)
    {
        var here = Position(t);
        leg = new Leg(here, here, t, 0);
    }

    private static double Distance((double X, double Y) a, (double X, double Y) b) =>
        Math.Sqrt((a.X - b.X) * (a.X - b.X) + (a.Y - b.Y) * (a.Y - b.Y));

    /// <summary>A random spot in the bounds at least MinimumHop of the diagonal from <paramref name="from"/>: the first of eight draws that is, else the farthest of them.</summary>
    private (double X, double Y) NextSpot((double X, double Y) from)
    {
        var hop = MinimumHop * Region.Diagonal;
        var best = from;
        var bestDistance = -1.0;
        for (var i = 0; i < 8; i++)
        {
            var x = Region.MinX + random.Next() * (Region.MaxX - Region.MinX);
            var y = Region.MinY + random.Next() * (Region.MaxY - Region.MinY);
            var d = Distance((x, y), from);
            if (d >= hop)
            {
                return (x, y);
            }
            if (d > bestDistance)
            {
                best = (x, y);
                bestDistance = d;
            }
        }
        return best;
    }
}
