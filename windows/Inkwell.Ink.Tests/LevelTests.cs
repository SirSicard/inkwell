// The map from the core's audio bands to the ink's voice: the Mac's InkLevels.level, the same
// floor and ceiling (tuned on the core's measured fixture levels).
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class LevelTests
{
    /// <summary>Bands whose powers add to <paramref name="db"/> dBFS, all in the mid band.</summary>
    private static double At(double db) => InkLevels.Level(0, (float)Math.Pow(10, db / 20), 0);

    [Fact]
    public void TheRoomReadsNothingAndOnlyTheLoudestSpeechSaturates()
    {
        Assert.Equal(0, At(-70), 3);
        Assert.Equal(0, At(InkLevels.FloorDb), 3);
        Assert.Equal(0.5, At(-37.5), 3);
        Assert.Equal(1, At(InkLevels.CeilingDb), 3);
        Assert.Equal(1, At(0), 3);
    }

    [Fact]
    public void TheBandsPowersAdd()
    {
        var one = (float)Math.Pow(10, -40.0 / 20);
        // Three equal bands are 10*log10(3) dB louder than one.
        var expected = (-40 + (10 * Math.Log10(3)) - InkLevels.FloorDb) / (InkLevels.CeilingDb - InkLevels.FloorDb);
        Assert.Equal(expected, InkLevels.Level(one, one, one), 6);
    }

    [Fact]
    public void SilenceAndNonsenseReadNothing()
    {
        Assert.Equal(0, InkLevels.Level(0, 0, 0));
        Assert.Equal(0, InkLevels.Level(float.NaN, 0, 0));
        Assert.Equal(0, InkLevels.Level(float.PositiveInfinity, 0, 0));
    }
}
