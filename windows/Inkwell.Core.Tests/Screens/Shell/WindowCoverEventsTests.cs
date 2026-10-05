// What WindowCover listens to and when it measures: only what can cover the window while it is
// uncovered, also what uncovers it while it is covered, nothing while it is hidden; only whole
// top-level windows count, and a burst of window events is measured once it settles, or half a
// second after it began.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class WindowCoverEventsTests
{
    private const uint Foreground = 0x0003;
    private const uint MoveSizeEnd = 0x000B;
    private const uint Destroy = 0x8001;
    private const uint Hide = 0x8003;
    private const uint Cloaked = 0x8017;
    private const int WholeWindow = 0;
    private const int Caret = -8;
    private const int Cursor = -9;

    [Fact]
    public void HiddenItHearsNothing()
    {
        Assert.Empty(WindowCoverEvents.Hooks(shown: false, covered: false));
        Assert.Empty(WindowCoverEvents.Hooks(shown: false, covered: true));
    }

    /// <summary>Uncovered: the front window, a move or resize ended, a window minimised or restored; not the mouse's capture between them (every press anywhere).</summary>
    [Fact]
    public void UncoveredItHearsWhatCanCoverIt()
    {
        Assert.Equal<(uint, uint)>([(0x0003, 0x0003), (0x000B, 0x000B), (0x0016, 0x0017)], WindowCoverEvents.Hooks(shown: true, covered: false));
    }

    /// <summary>
    /// Covered: also only what uncovers it, a window destroyed, hidden or cloaked; not one shown,
    /// uncloaked or moved (a location change comes with every move of the mouse), and never the
    /// mouse's capture.
    /// </summary>
    [Fact]
    public void CoveredItAlsoHearsWhatUncoversIt()
    {
        Assert.Equal<(uint, uint)>(
            [(0x0003, 0x0003), (0x000B, 0x000B), (0x0016, 0x0017), (0x8001, 0x8001), (0x8003, 0x8003), (0x8017, 0x8017)],
            WindowCoverEvents.Hooks(shown: true, covered: true));
    }

    [Fact]
    public void OnlyWholeTopLevelWindowsCount()
    {
        Assert.True(WindowCoverEvents.Measures(Hide, WholeWindow, 0, topLevel: true));
        Assert.True(WindowCoverEvents.Measures(Destroy, WholeWindow, 0, topLevel: true));
        Assert.True(WindowCoverEvents.Measures(Foreground, WholeWindow, 0, topLevel: true));
        Assert.False(WindowCoverEvents.Measures(Hide, Cursor, 0, topLevel: false)); // the mouse pointer hidden
        Assert.False(WindowCoverEvents.Measures(Hide, Caret, 0, topLevel: false));
        Assert.False(WindowCoverEvents.Measures(Destroy, WholeWindow, 0, topLevel: false)); // a child window
        Assert.False(WindowCoverEvents.Measures(Hide, WholeWindow, 4, topLevel: true)); // a part of one
    }

    [Fact]
    public void ABurstOfWindowEventsSettlesFirst()
    {
        Assert.True(WindowCoverEvents.Settles(Hide));
        Assert.True(WindowCoverEvents.Settles(Destroy));
        Assert.True(WindowCoverEvents.Settles(Cloaked));
        Assert.False(WindowCoverEvents.Settles(Foreground)); // measured at once, as before
        Assert.False(WindowCoverEvents.Settles(MoveSizeEnd));
        Assert.True(WindowCoverEvents.Settle > TimeSpan.Zero);
    }

    /// <summary>A burst (an app closing its windows) is measured when it settles, and never later than half a second after it began.</summary>
    [Fact]
    public void ABurstWaitsAtMostHalfASecond()
    {
        Assert.Equal(WindowCoverEvents.Settle, WindowCoverEvents.SettleDelay(TimeSpan.Zero));
        Assert.Equal(TimeSpan.FromMilliseconds(50), WindowCoverEvents.SettleDelay(TimeSpan.FromMilliseconds(450)));
        Assert.Equal(TimeSpan.Zero, WindowCoverEvents.SettleDelay(TimeSpan.FromMilliseconds(700)));
    }
}
