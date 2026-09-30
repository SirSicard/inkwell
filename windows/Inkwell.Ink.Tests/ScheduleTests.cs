// When the ink draws: architecture rule 9 ("draw nothing when idle") as a surface's decisions. The
// Mac's ScheduleTests, one for one.
using Inkwell.Ink;
using Xunit;

namespace Inkwell.Ink.Tests;

public sealed class ScheduleTests
{
    [Fact]
    public void IdleOnScreenDrawsOneStillFrameAndNeverRunsTheClock()
    {
        var s = new InkSchedule();
        Assert.Equal(InkAction.DrawStill, s.SetOnScreen(true));
        Assert.Equal(InkAction.Nothing, s.Update());
        Assert.Equal(InkAction.Nothing, s.Update());
        Assert.False(s.ClockRunning);
    }

    [Fact]
    public void ALiveStateRunsTheClockAndIdleStopsItWithOneStillFrame()
    {
        var s = new InkSchedule();
        s.SetOnScreen(true);
        Assert.Equal(InkAction.StartClock, s.SetState(InkState.Meeting));
        Assert.True(s.ClockRunning);
        Assert.Equal(InkAction.Nothing, s.SetState(InkState.Blotting));
        Assert.Equal(InkAction.StopClockAndDrawStill, s.SetState(InkState.Idle));
        Assert.False(s.ClockRunning);
        Assert.Equal(InkAction.Nothing, s.Update());
    }

    [Fact]
    public void NothingDrawsOffScreen()
    {
        var s = new InkSchedule();
        Assert.Equal(InkAction.Nothing, s.SetState(InkState.Dictating));
        Assert.Equal(InkAction.Nothing, s.SetState(InkState.Idle));
        Assert.Equal(InkAction.Nothing, s.Invalidate());
        Assert.Equal(InkAction.DrawStill, s.SetOnScreen(true));
    }

    [Fact]
    public void HidingALiveInkStopsTheClockAndShowingItStartsItAgain()
    {
        var s = new InkSchedule();
        s.SetOnScreen(true);
        s.SetState(InkState.Meeting);
        Assert.Equal(InkAction.StopClock, s.SetOnScreen(false));
        Assert.Equal(InkAction.StartClock, s.SetOnScreen(true));
    }

    [Fact]
    public void AnIdleInkThatWasHiddenIsNotRedrawnWhenShown()
    {
        var s = new InkSchedule();
        s.SetOnScreen(true);
        s.SetOnScreen(false);
        Assert.Equal(InkAction.Nothing, s.SetOnScreen(true));
    }

    [Fact]
    public void GoingIdleWhileHiddenDrawsTheStillFrameWhenShown()
    {
        var s = new InkSchedule();
        s.SetOnScreen(true);
        s.SetState(InkState.Meeting);
        s.SetOnScreen(false);
        Assert.Equal(InkAction.Nothing, s.SetState(InkState.Idle));
        Assert.Equal(InkAction.DrawStill, s.SetOnScreen(true));
    }

    [Fact]
    public void AnimationEffectsOffShowsEachStateAsOneStillFrame()
    {
        var s = new InkSchedule();
        s.SetOnScreen(true);
        Assert.Equal(InkAction.Nothing, s.SetReduceMotion(true));
        Assert.Equal(InkAction.DrawStill, s.SetState(InkState.Dictating));
        Assert.Equal(InkAction.DrawStill, s.SetState(InkState.Meeting));
        Assert.False(s.ClockRunning);
        Assert.Equal(InkAction.StartClock, s.SetReduceMotion(false));
        Assert.Equal(InkAction.StopClockAndDrawStill, s.SetReduceMotion(true));
    }

    [Fact]
    public void AResizeWhileIdleRedrawsOnceAndWhileLiveNotAtAll()
    {
        var s = new InkSchedule();
        s.SetOnScreen(true);
        Assert.Equal(InkAction.DrawStill, s.Invalidate());
        Assert.Equal(InkAction.Nothing, s.Update());
        s.SetState(InkState.Dictating);
        Assert.Equal(InkAction.Nothing, s.Invalidate());
    }
}
