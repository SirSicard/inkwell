// The Library's kind filters wrap between chips, never inside one: rows break where the next chip
// would not fit.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

public class WrapLayoutTests
{
    // All, Meetings, Dictations, Files as the Library measured them (30 high).
    private static readonly (double, double)[] Chips = [(46, 30), (88, 30), (92, 30), (58, 30)];

    [Fact]
    public void ChipsThatFitStayOnOneRow()
    {
        var (places, width, height) = WrapLayout.Place(Chips, 400, 6, 6);
        Assert.Equal([(0d, 0d), (52d, 0d), (146d, 0d), (244d, 0d)], places);
        Assert.Equal(302, width);
        Assert.Equal(30, height);
    }

    /// <summary>
    /// The list column's 236 px: Dictations would end at 238, so it starts a second row whole,
    /// and Files follows it there, instead of being cut off.
    /// </summary>
    [Fact]
    public void AChipThatWouldNotFitStartsTheNextRow()
    {
        var (places, width, height) = WrapLayout.Place(Chips, 236, 6, 6);
        Assert.Equal([(0d, 0d), (52d, 0d), (0d, 36d), (98d, 36d)], places);
        Assert.Equal(156, width);
        Assert.Equal(66, height);
    }

    [Fact]
    public void ExactlyFullRowsDoNotWrap()
    {
        var (places, _, height) = WrapLayout.Place([(100, 20), (100, 20)], 206, 6, 4);
        Assert.Equal([(0d, 0d), (106d, 0d)], places);
        Assert.Equal(20, height);
    }

    [Fact]
    public void AnItemWiderThanARowGetsARowOfItsOwn()
    {
        var (places, _, height) = WrapLayout.Place([(50, 20), (300, 20), (50, 20)], 200, 6, 4);
        Assert.Equal([(0d, 0d), (0d, 24d), (0d, 48d)], places);
        Assert.Equal(68, height);
    }

    [Fact]
    public void CollapsedItemsTakeNoPlaceAndNoSpacing()
    {
        var (places, width, _) = WrapLayout.Place([(40, 20), (0, 0), (40, 20)], 200, 6, 4);
        Assert.Equal(46d, places[2].X);
        Assert.Equal(86, width);
        Assert.Equal(0, WrapLayout.Place([(0, 0)], 200, 6, 4).Height);
        Assert.Equal(0, WrapLayout.Place([], 200, 6, 4).Height);
    }
}
