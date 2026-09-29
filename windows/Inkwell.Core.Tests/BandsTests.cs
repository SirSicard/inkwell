// The ink's levels from C#: ink_bands_read and ink_far_bands_read through InkSession, as the ink
// reads them once per frame. They answer with or without a running core (zeros before any audio),
// never throw, and never go backwards.
//
// Needs the core's DLL, as SmokeTests.
using Xunit;

namespace Inkwell.Core.Tests;

public sealed class BandsTests
{
    [Fact]
    public void TheBandsAreReadWithOrWithoutACore()
    {
        foreach (var read in new Func<AudioBands>[] { InkSession.Bands, InkSession.FarBands })
        {
            var first = read();
            var second = read();
            foreach (var band in new[] { first.Low, first.Mid, first.High })
            {
                Assert.True(float.IsFinite(band) && band >= 0, $"{band}");
            }
            Assert.True(second.Published >= first.Published);
        }
    }
}
