// A screen's clock runs only while its screen is loaded, the window is on screen and something moves.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class ScreenClockTests
{
    [Fact]
    public void AClockRunsOnlyWhileLoadedOnScreenAndMoving()
    {
        var presence = new WindowPresence();
        presence.Update(visible: true, minimized: false, occlusionVisible: true);
        Assert.True(ScreenClock.Runs(loaded: true, presence, moving: true));
        Assert.False(ScreenClock.Runs(loaded: false, presence, moving: true));
        Assert.False(ScreenClock.Runs(loaded: true, presence, moving: false));

        // Hidden to the tray, or minimised: nothing ticks, though the audio plays on.
        presence.Update(visible: false, minimized: false, occlusionVisible: true);
        Assert.False(ScreenClock.Runs(loaded: true, presence, moving: true));
        presence.Update(visible: true, minimized: true, occlusionVisible: true);
        Assert.False(ScreenClock.Runs(loaded: true, presence, moving: true));

        presence.Update(visible: true, minimized: false, occlusionVisible: true);
        Assert.True(ScreenClock.Runs(loaded: true, presence, moving: true));
    }
}
