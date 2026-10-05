// What WindowCover listens to and when it measures: only what can cover the window while it is
// uncovered, also what can uncover it while it is covered, nothing while it is hidden; only whole
// top-level windows count, and a burst of window events is measured once it settles.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class WindowCoverEventsTests
{
    private const uint Foreground = 0x0003;
    private const uint MoveSizeEnd = 0x000B;
    private const uint Destroy = 0x8001;
    private const uint Hide = 0x8003;
    private const uint LocationChange = 0x800B;
    private const uint Uncloaked = 0x8018;
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

    /// <summary>Covered: also a window destroyed, shown or hidden, moved by code, cloaked or uncloaked; still never the mouse's capture.</summary>
    [Fact]
    public void CoveredItAlsoHearsWhatCanUncoverIt()
    {
        var hooks = WindowCoverEvents.Hooks(shown: true, covered: true);
        Assert.Contains((0x0003u, 0x0003u), hooks);
        Assert.Contains((0x8001u, 0x8003u), hooks);
        Assert.Contains((0x800Bu, 0x800Bu), hooks);
        Assert.Contains((0x8017u, 0x8018u), hooks);
        Assert.All(hooks, hook => Assert.False(hook.Min <= 0x0009 && hook.Max >= 0x0008));
    }

    [Fact]
    public void OnlyWholeTopLevelWindowsCount()
    {
        Assert.True(WindowCoverEvents.Measures(LocationChange, WholeWindow, 0, topLevel: true));
        Assert.True(WindowCoverEvents.Measures(Destroy, WholeWindow, 0, topLevel: true));
        Assert.True(WindowCoverEvents.Measures(Foreground, WholeWindow, 0, topLevel: true));
        Assert.False(WindowCoverEvents.Measures(LocationChange, Cursor, 0, topLevel: false)); // the mouse moving
        Assert.False(WindowCoverEvents.Measures(LocationChange, Caret, 0, topLevel: false));
        Assert.False(WindowCoverEvents.Measures(LocationChange, WholeWindow, 0, topLevel: false)); // a child window
        Assert.False(WindowCoverEvents.Measures(Hide, WholeWindow, 4, topLevel: true)); // a part of one
    }

    [Fact]
    public void ABurstOfWindowEventsSettlesFirst()
    {
        Assert.True(WindowCoverEvents.Settles(LocationChange));
        Assert.True(WindowCoverEvents.Settles(Destroy));
        Assert.True(WindowCoverEvents.Settles(Uncloaked));
        Assert.False(WindowCoverEvents.Settles(Foreground)); // measured at once, as before
        Assert.False(WindowCoverEvents.Settles(MoveSizeEnd));
        Assert.True(WindowCoverEvents.Settle > TimeSpan.Zero);
    }
}
