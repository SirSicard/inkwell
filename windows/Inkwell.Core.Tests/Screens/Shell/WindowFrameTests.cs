// The window inside the work area: moved the least, shrunk only when bigger, untouched when inside.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class WindowFrameTests
{
    // A 1920 x 1080 display whose taskbar takes the bottom 48 pixels.
    private static readonly WindowFrame Work = new(0, 0, 1920, 1032);

    [Fact]
    public void AFrameInsideStaysAsItIs()
    {
        var frame = new WindowFrame(100, 80, 1200, 800);
        Assert.Equal(frame, frame.Fitted(Work));
        Assert.Equal(Work, Work.Fitted(Work));
    }

    [Fact]
    public void AFrameOffAnEdgeMovesBackTheLeast()
    {
        Assert.Equal(new WindowFrame(0, 80, 1200, 800), new WindowFrame(-300, 80, 1200, 800).Fitted(Work));
        Assert.Equal(new WindowFrame(720, 80, 1200, 800), new WindowFrame(1500, 80, 1200, 800).Fitted(Work));
        // Under the taskbar: up until its bottom edge is the work area's.
        Assert.Equal(new WindowFrame(100, 232, 1200, 800), new WindowFrame(100, 600, 1200, 800).Fitted(Work));
        Assert.Equal(new WindowFrame(100, 0, 1200, 800), new WindowFrame(100, -50, 1200, 800).Fitted(Work));
    }

    /// <summary>Left on a display that was unplugged: brought onto this one whole.</summary>
    [Fact]
    public void AFrameOnAMissingDisplayComesBackWhole() =>
        Assert.Equal(new WindowFrame(720, 232, 1200, 800), new WindowFrame(3000, 1500, 1200, 800).Fitted(Work));

    [Fact]
    public void AFrameBiggerThanTheWorkAreaIsShrunkToIt()
    {
        Assert.Equal(Work, new WindowFrame(-100, -100, 2560, 1440).Fitted(Work));
        Assert.Equal(new WindowFrame(0, 100, 1920, 900), new WindowFrame(50, 100, 2400, 900).Fitted(Work));
    }

    /// <summary>A work area that does not start at 0,0 (a second display to the left, above or at 150%).</summary>
    [Fact]
    public void AWorkAreaAwayFromTheOriginIsHonoured()
    {
        var left = new WindowFrame(-2560, 200, 2560, 1400);
        Assert.Equal(new WindowFrame(-1200, 800, 1200, 800), new WindowFrame(100, 900, 1200, 800).Fitted(left));
        Assert.Equal(new WindowFrame(-2560, 200, 1200, 800), new WindowFrame(-3000, 0, 1200, 800).Fitted(left));
    }
}
