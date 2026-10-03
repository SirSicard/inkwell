// Where the main window's orb goes, as the Mac's WanderTests, ScheduleTests' glide and
// WanderViewTests: a seeded path, so each check sees the same one. Live it wanders slowly from spot
// to spot; at rest it glides to a new spot when asked, then holds still, and the surface draws the
// glide on the clock for its length only.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class WanderTests
{
    private static readonly OrbWander.Bounds Bounds = OrbWander.Main;

    private static OrbWander Wander(uint seed = 7) => new(Bounds, (0.56, 0.26), InkRandom.Seeded(seed));

    private static double Distance((double X, double Y) a, (double X, double Y) b) =>
        Math.Sqrt((a.X - b.X) * (a.X - b.X) + (a.Y - b.Y) * (a.Y - b.Y));

    [Fact]
    public void TheRegionIsTheMacs() => Assert.Equal(new OrbWander.Bounds(0.46, 0.68, 0.18, 0.36), OrbWander.Main);

    /// <summary>
    /// Ten minutes of live wander at 60 frames a second: always inside the bounds, never faster than
    /// a slow drift (no jump between legs), and over the minutes it visits much of the region.
    /// </summary>
    [Fact]
    public void LiveItWandersSlowlyAndSmoothlyInsideTheBounds()
    {
        var w = Wander();
        var last = w.Position(0);
        var fastest = 0.0;
        var (minX, maxX, minY, maxY) = (last.X, last.X, last.Y, last.Y);
        for (var frame = 1; frame <= 60 * 600; frame++)
        {
            var t = frame / 60.0;
            w.Wander(t);
            var p = w.Position(t);
            Assert.True(Bounds.Contains(p), $"{p} at {t} s");
            fastest = Math.Max(fastest, Distance(p, last) * 60);
            (minX, maxX, minY, maxY) = (Math.Min(minX, p.X), Math.Max(maxX, p.X), Math.Min(minY, p.Y), Math.Max(maxY, p.Y));
            last = p;
        }
        Assert.True(fastest < 0.025, $"{fastest} per second, in fractions of the view");
        Assert.True(maxX - minX > 0.6 * (Bounds.MaxX - Bounds.MinX));
        Assert.True(maxY - minY > 0.6 * (Bounds.MaxY - Bounds.MinY));
    }

    [Fact]
    public void TheSameSeedTakesTheSamePath()
    {
        var (a, b, c) = (Wander(3), Wander(3), Wander(4));
        var differs = false;
        for (var frame = 0; frame <= 60 * 60; frame += 30)
        {
            var t = frame / 60.0;
            a.Wander(t);
            b.Wander(t);
            c.Wander(t);
            Assert.Equal(a.Position(t), b.Position(t));
            differs |= a.Position(t) != c.Position(t);
        }
        Assert.True(differs, "another seed, another path");
    }

    /// <summary>At rest: asked to move, it glides over RestGlide to a new spot well away from the old one, eased, then holds it.</summary>
    [Fact]
    public void AtRestItGlidesToANewSpotThenHoldsStill()
    {
        var w = Wander();
        var from = w.Position(10);
        Assert.False(w.IsMoving(10)); // it starts still
        w.Move(10, animated: true);
        Assert.True(w.IsMoving(10.01));
        Assert.True(w.IsMoving(10 + OrbWander.RestGlide - 0.01));
        Assert.False(w.IsMoving(10 + OrbWander.RestGlide));
        var to = w.Position(10 + OrbWander.RestGlide);
        Assert.True(Bounds.Contains(to));
        Assert.True(Distance(from, to) >= OrbWander.MinimumHop * Bounds.Diagonal - 1e-9, "a new spot, not a nudge");
        // Eased: the first tenth of the time covers far less than a tenth of the way.
        Assert.True(Distance(from, w.Position(10 + OrbWander.RestGlide * 0.1)) < 0.05 * Distance(from, to));
        Assert.Equal(to, w.Position(500)); // and it holds there
    }

    /// <summary>With motion off it takes the new spot at once: nothing to animate.</summary>
    [Fact]
    public void WithoutMotionItTakesTheNewSpotAtOnce()
    {
        var w = Wander();
        var from = w.Position(10);
        w.Move(10, animated: false);
        Assert.False(w.IsMoving(10));
        Assert.NotEqual(from, w.Position(10));
        Assert.True(Bounds.Contains(w.Position(10)));
    }

    /// <summary>Going to rest mid-leg holds the orb where it is: no jump back to a rest spot.</summary>
    [Fact]
    public void HoldingStopsItWhereItIs()
    {
        var w = Wander();
        for (var frame = 0; frame <= 60 * 5; frame++)
        {
            w.Wander(frame / 60.0);
        }
        var here = w.Position(5);
        w.Hold(5);
        Assert.False(w.IsMoving(5));
        Assert.Equal(here, w.Position(5));
        Assert.Equal(here, w.Position(60));
    }

    [Fact]
    public void AStartOutsideTheBoundsIsPulledIn() =>
        Assert.Equal((0.68, 0.18), new OrbWander(Bounds, (0.9, 0.05), InkRandom.Seeded(1)).Position(0));

    /// <summary>
    /// A glide at rest runs the clock for its length only, then the settled frame at the new spot;
    /// never off screen or with motion off (the Mac's ScheduleTests).
    /// </summary>
    [Fact]
    public void AGlideAtRestRunsTheClockOnlyWhileItLasts()
    {
        var s = new InkSchedule();
        _ = s.SetOnScreen(true);
        Assert.Equal(InkAction.StartClock, s.SetGliding(true));
        Assert.Equal(InkAction.StopClockAndDrawStill, s.SetGliding(false)); // arrived: one still frame there
        Assert.Equal(InkAction.Nothing, s.Update()); // and nothing after it
        Assert.False(s.ClockRunning);

        Assert.Equal(InkAction.StartClock, s.SetGliding(true));
        Assert.Equal(InkAction.StopClockAndDrawStill, s.SetReduceMotion(true)); // motion stilled mid-glide: it holds
        Assert.False(s.Gliding);
        Assert.Equal(InkAction.Nothing, s.SetReduceMotion(false)); // and no glide resumes
        _ = s.SetReduceMotion(true);
        Assert.Equal(InkAction.Nothing, s.SetGliding(true)); // motion off: no glide
        _ = s.SetGliding(false);
        _ = s.SetReduceMotion(false);
        _ = s.SetGliding(true);
        Assert.Equal(InkAction.StopClock, s.SetOnScreen(false)); // hidden mid-glide
        Assert.False(s.Gliding);
        Assert.Equal(InkAction.Nothing, s.SetGliding(true)); // off screen: nothing moves
    }
}

/// <summary>The wander through a real surface, clock and pipeline (WARP), as the Mac's WanderViewTests.</summary>
public sealed class WanderSurfaceTests
{
    private static readonly InkPipelineLoader Loader = new(() => new InkPipeline(TestPipeline.Adapter));

    private static (InkSurface Surface, OffscreenTarget Target) Make(InkClock clock)
    {
        Assert.Null(Loader.Wait().Failure);
        var target = new OffscreenTarget();
        var surface = new InkSurface(target, Loader, clock) { AssumeReduceMotion = false, WanderRandom = InkRandom.Seeded(5) };
        surface.WanderBounds = OrbWander.Main;
        var (w, h) = InkSurface.CanvasPixels(160, 100, 1);
        surface.SetCanvas(w, h);
        return (surface, target);
    }

    /// <summary>
    /// Coming on screen at rest it glides to a new spot over about 2.4 s on the clock, then draws
    /// nothing more: a resting window left alone draws no frames. A change of screen moves it
    /// again; a held orb stays.
    /// </summary>
    [Fact]
    public void AtRestItGlidesWhenShownThenDrawsNothing()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            var clock = new InkClock(ui.Post);
            var (surface, target) = Make(clock);
            using var _ = target;
            using var __ = surface;
            var home = surface.OrbCentre;

            surface.SetOnScreen(true);
            Assert.True(surface.IsGliding);
            Assert.True(surface.IsAnimating);
            ui.Pump(OrbWander.RestGlide + 0.4);
            Assert.False(surface.IsGliding);
            Assert.False(surface.IsAnimating);
            Assert.Equal(0, clock.ThreadCount);
            var glided = surface.FramesDrawn;
            Assert.True(glided > 20, $"{glided} frames for the glide");
            var spot = surface.OrbCentre;
            Assert.NotEqual(home, spot);
            Assert.True(OrbWander.Main.Contains(spot));
            ui.Pump(0.5);
            Assert.Equal(glided, surface.FramesDrawn); // at rest, nothing more

            // Activated soon after a move: it stays.
            surface.Activated();
            Assert.False(surface.IsGliding);
            // Activated after RestInterval in one spot: it moves.
            surface.RestInterval = 0;
            surface.Activated();
            Assert.True(surface.IsGliding);
            ui.Pump(OrbWander.RestGlide + 0.4);
            Assert.False(surface.IsGliding);

            // Held: no new spot for a change of screen.
            surface.HoldsStill = true;
            var held = surface.OrbCentre;
            surface.MoveAtRest();
            Assert.False(surface.IsGliding);
            Assert.Equal(held, surface.OrbCentre);
            surface.HoldsStill = false;
            surface.MoveAtRest();
            Assert.True(surface.IsGliding);
        }
    }

    /// <summary>With motion stilled it takes each new spot in its one still frame, without a glide; Settled answers at once.</summary>
    [Fact]
    public void StilledItMovesInOneStillFrame()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            var clock = new InkClock(ui.Post);
            var (surface, target) = Make(clock);
            using var _ = target;
            using var __ = surface;
            surface.AssumeReduceMotion = true;
            var home = surface.OrbCentre;
            surface.SetOnScreen(true);
            Assert.False(surface.IsGliding);
            Assert.Equal(1, surface.FramesDrawn);
            Assert.NotEqual(home, surface.OrbCentre);
            Assert.True(surface.Settled(TestContext.Current.CancellationToken).IsCompleted);
            surface.MoveAtRest();
            Assert.Equal(2, surface.FramesDrawn);
            ui.Pump(0.3);
            Assert.Equal(2, surface.FramesDrawn);
        }
    }

    /// <summary>Settled waits for a glide under way, then answers with the orb where it arrived.</summary>
    [Fact]
    public void SettledWaitsForTheGlide()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            var clock = new InkClock(ui.Post);
            var (surface, target) = Make(clock);
            using var _ = target;
            using var __ = surface;
            var waiting = surface.Settled(TestContext.Current.CancellationToken);
            Assert.False(waiting.IsCompleted); // not on screen yet
            surface.HoldsStill = true;
            surface.SetOnScreen(true);
            Assert.True(surface.IsGliding); // held, it still takes the spot it takes on coming on screen
            Assert.False(waiting.IsCompleted);
            ui.Pump(OrbWander.RestGlide + 0.4);
            Assert.True(waiting.IsCompleted);
            Assert.False(surface.IsGliding);
        }
    }
}
