// The window's orb dims while anything is live, so the text over it reads: a fade toward a target
// opacity over 0.8 s, read at each frame's time.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class OrbFadeTests
{
    /// <summary>Behind the main window's text: 70 % at rest, 30 % live, never above 45 % under High Contrast (the Mac's OrbLayer.opacity).</summary>
    [Fact]
    public void BehindTextItRestsAtSeventyAndDimsLiveAndUnderHighContrast()
    {
        Assert.Equal(0.7f, OrbFade.BehindText(live: false, dimmed: false));
        Assert.Equal(0.3f, OrbFade.BehindText(live: true, dimmed: false));
        Assert.Equal(0.45f, OrbFade.BehindText(live: false, dimmed: true));
        Assert.Equal(0.3f, OrbFade.BehindText(live: true, dimmed: true));
    }

    /// <summary>The user's strength is the opacity at rest; live keeps 3/7 of it up to the default, then climbs to 80 % at the strongest; High Contrast still caps it.</summary>
    [Fact]
    public void BehindTextFollowsTheUsersStrength()
    {
        Assert.Equal(1f, OrbFade.BehindText(live: false, dimmed: false, rest: 1f));
        Assert.Equal(0.8f, OrbFade.BehindText(live: true, dimmed: false, rest: 1f), 5);
        Assert.Equal(0.3f, OrbFade.BehindText(live: true, dimmed: false, rest: 0.7f), 5);
        Assert.Equal(0.55f, OrbFade.BehindText(live: true, dimmed: false, rest: 0.85f), 5);
        Assert.Equal(0.3f * 0.4f / 0.7f, OrbFade.BehindText(live: true, dimmed: false, rest: 0.4f), 5);
        Assert.Equal(0.1f, OrbFade.BehindText(live: false, dimmed: false, rest: 0.1f));
        Assert.Equal(0.1f, OrbFade.BehindText(live: false, dimmed: false, rest: 0f)); // held at the slider's least
        Assert.Equal(0.45f, OrbFade.BehindText(live: false, dimmed: true, rest: 1f));
    }

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
