// The Drop's window: styles that keep focus where the user put it, shown and hidden without ever
// becoming the foreground or active window, drawing while live and nothing while hidden. Whether
// keystrokes really stay in another app needs a person at the desktop (windows/S3.4-CHECKLIST.md).
using Inkwell.Ink;
using TerraFX.Interop.Windows;
using Xunit;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink.Tests;

public sealed class DropTests
{
    private static readonly InkPipelineLoader Loader = new(() => new InkPipeline(TestPipeline.Adapter));

    /// <summary>DXGI_ERROR_NOT_CURRENTLY_AVAILABLE: no compositor in this session (SSH, a service).</summary>
    private const string NoCompositor = "(0x887A0022)";

    private static readonly InkState[] LiveStates = [InkState.Dictating, InkState.Meeting, InkState.Blotting, InkState.Problem];

    [Fact]
    public void TheDropNeverActivates()
    {
        lock (TestPipeline.Lock)
        {
            Assert.Null(Loader.Wait().Failure);
            var ui = new UiThread();
            using var drop = new DropWindow(Loader, new InkClock(ui.Post));
            var hwnd = (HWND)drop.Handle;

            var ex = (uint)GetWindowLongW(hwnd, GWL.GWL_EXSTYLE);
            foreach (var flag in new[] { WS.WS_EX_NOACTIVATE, WS.WS_EX_TOPMOST, WS.WS_EX_TOOLWINDOW, WS.WS_EX_NOREDIRECTIONBITMAP })
            {
                Assert.True((ex & (uint)flag) != 0, $"extended style 0x{flag:X} is set");
            }
            Assert.False(Visible(hwnd));

            var foreground = GetForegroundWindow();
            foreach (var state in LiveStates)
            {
                drop.Update(state);
                ui.Pump(0.1);
                Assert.True(drop.IsShown);
                Assert.True(Visible(hwnd));
                Assert.Equal(state, drop.Surface.State);
                Assert.Equal(DropText.For(state), drop.ShownText);
                Assert.NotEqual(hwnd, GetForegroundWindow());
                Assert.NotEqual(hwnd, GetActiveWindow());
                Assert.Equal(foreground, GetForegroundWindow());
            }
            drop.Update(InkState.Idle);
            Assert.False(drop.IsShown);
            Assert.False(Visible(hwnd));
            Assert.Null(drop.ShownText);
            Assert.Equal(foreground, GetForegroundWindow());
        }
    }

    [Fact]
    public void TheDropDrawsWhileLiveAndNothingWhileHidden()
    {
        lock (TestPipeline.Lock)
        {
            Assert.Null(Loader.Wait().Failure);
            var ui = new UiThread();
            var clock = new InkClock(ui.Post);
            using var drop = new DropWindow(Loader, clock);
            drop.Surface.AssumeReduceMotion = false;
            ui.Pump(0.3);
            Assert.Equal(0, drop.Surface.FramesDrawn);

            drop.Update(InkState.Dictating);
            Assert.SkipWhen(drop.Surface.Failure?.EndsWith(NoCompositor, StringComparison.Ordinal) == true,
                $"no compositor in this session: {drop.Surface.Failure}");
            foreach (var state in LiveStates)
            {
                drop.Update(state);
                ui.Pump(0.2);
                Assert.Null(drop.Surface.Failure);
            }
            Assert.True(drop.Surface.FramesDrawn > 10, $"live frames: {drop.Surface.FramesDrawn}");

            drop.Update(InkState.Idle);
            var frames = drop.Surface.FramesDrawn;
            var all = InkFrames.Count;
            ui.Pump(1.0);
            Assert.Equal(frames, drop.Surface.FramesDrawn);
            Assert.Equal(all, InkFrames.Count);
            Assert.Equal(0, clock.ThreadCount);
        }
    }

    [Fact]
    public void WhenTheShaderDoesNotCompileTheDropShowsItsPlainFallback()
    {
        var ui = new UiThread();
        var loader = new InkPipelineLoader(() => new InkPipeline(InkAdapter.Warp, "not a shader"));
        Assert.True(loader.Wait().Permanent);
        using var drop = new DropWindow(loader, new InkClock(ui.Post));
        string? reported = null;
        drop.Surface.FailureChanged += m => reported = m;
        ui.Pump(0.2);
        var foreground = GetForegroundWindow();

        drop.Update(InkState.Meeting);
        ui.Pump(0.2);
        Assert.True(drop.ShowsFallback, "the recording indicator is never blank");
        var fallback = (HWND)drop.FallbackHandle;
        Assert.True(Visible(fallback));
        var ex = (uint)GetWindowLongW(fallback, GWL.GWL_EXSTYLE);
        foreach (var flag in new[] { WS.WS_EX_NOACTIVATE, WS.WS_EX_TOPMOST, WS.WS_EX_TOOLWINDOW })
        {
            Assert.True((ex & (uint)flag) != 0, $"fallback extended style 0x{flag:X} is set");
        }
        Assert.Equal(foreground, GetForegroundWindow());
        Assert.NotEqual(fallback, GetActiveWindow());
        Assert.Equal(0, drop.Surface.FramesDrawn);

        drop.Update(InkState.Problem);
        Assert.True(drop.ShowsFallback);
        Assert.Equal(DropText.For(InkState.Problem), drop.ShownText);

        drop.Update(InkState.Idle);
        Assert.False(drop.ShowsFallback);
        Assert.False(Visible(fallback));
        Assert.Equal(foreground, GetForegroundWindow());
    }

    [Fact]
    public void TheDropRecoversFromALostDevice()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            var loader = new InkPipelineLoader(() => new InkPipeline(TestPipeline.Adapter));
            using var drop = new DropWindow(loader, new InkClock(ui.Post));
            drop.Surface.AssumeReduceMotion = false;
            ui.Pump(0.5);
            drop.Update(InkState.Dictating);
            ui.Pump(0.3);
            Assert.SkipWhen(drop.Surface.Failure?.Contains(NoCompositor, StringComparison.Ordinal) == true,
                $"no compositor in this session: {drop.Surface.Failure}");
            Assert.Null(drop.Surface.Failure);
            var before = drop.Surface.FramesDrawn;

            // What a TDR does: the next Present returns DXGI_ERROR_DEVICE_REMOVED.
            drop.FailNextPresent = FlakyTarget.DeviceRemoved;
            ui.Pump(0.1);
            Assert.NotNull(drop.Surface.Failure);
            Assert.True(drop.ShowsFallback, "the plain panel shows while the device comes back");
            var until = DateTime.UtcNow.AddSeconds(15);
            while (drop.Surface.Failure is not null && DateTime.UtcNow < until)
            {
                ui.Pump(0.05);
            }
            Assert.Null(drop.Surface.Failure);
            Assert.Equal(2, loader.Compiles);
            var after = drop.Surface.FramesDrawn;
            ui.Pump(0.5);
            Assert.True(drop.Surface.FramesDrawn > after + 5, "live again after the recreate");
            Assert.True(after > before);
            Assert.False(drop.ShowsFallback);
            drop.Update(InkState.Idle);
            loader.Outcome?.Pipeline?.Dispose();
        }
    }

    /// <summary>
    /// WS_VISIBLE. IsWindowVisible also asks the desktop, which is never visible in a session
    /// without one (SSH, CI), so it would say no there whatever the window does.
    /// </summary>
    private static bool Visible(HWND hwnd) => ((uint)GetWindowLongW(hwnd, GWL.GWL_STYLE) & WS.WS_VISIBLE) != 0;

    [Fact]
    public void EachStateSaysWhatTheMacSays()
    {
        Assert.Equal(new DropText("Dictating", "Listening"), DropText.For(InkState.Dictating));
        Assert.Equal(new DropText("\u25CF REC", "Recording this meeting", DropTone.Recording), DropText.For(InkState.Meeting));
        Assert.Equal(new DropText("Blotting", "The final pass"), DropText.For(InkState.Blotting));
        Assert.Equal(new DropText("Far end silent", "Nothing is arriving from the call", DropTone.Alert), DropText.For(InkState.Problem));
    }
}
