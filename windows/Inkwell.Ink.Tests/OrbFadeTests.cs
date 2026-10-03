// The window's orb dims while anything is live, so the text over it reads: a fade toward a target
// opacity over 0.8 s, read at each frame's time.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class OrbFadeTests
{
    [Fact]
    public void ItFadesToItsTargetOverEightTenthsOfASecondAndBack()
    {
        var fade = new OrbFade();
        Assert.Equal(1f, fade.ValueAt(0));
        Assert.False(fade.Fading(0));

        fade.FadeTo(0.3f, now: 10, animate: true);
        Assert.Equal(1f, fade.ValueAt(10), 3);
        Assert.Equal(0.65f, fade.ValueAt(10.4), 2); // halfway
        Assert.Equal(0.3f, fade.ValueAt(10.8), 3);
        Assert.Equal(0.3f, fade.ValueAt(99), 3);
        Assert.True(fade.Fading(10.5));
        Assert.False(fade.Fading(10.8));

        // Turned back halfway: from where it is, not from the end.
        fade.FadeTo(1f, now: 20, animate: true);
        fade.FadeTo(0.3f, now: 20.4, animate: true);
        Assert.Equal(0.65f, fade.ValueAt(20.4), 2);
        Assert.Equal(0.3f, fade.ValueAt(21.2), 3);
    }

    [Fact]
    public void WithoutAnimationsItJumps()
    {
        var fade = new OrbFade();
        fade.FadeTo(0.3f, now: 5, animate: false);
        Assert.Equal(0.3f, fade.ValueAt(5), 3);
        Assert.False(fade.Fading(5));
        Assert.Equal(0.3f, fade.Target);
    }
}
