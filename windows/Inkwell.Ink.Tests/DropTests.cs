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
    public unsafe void WhenTheShaderDoesNotCompileTheDropShowsItsPlainFallback()
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

        // A DPI or display change moves and scales the fallback with the Drop.
        SetWindowPos(fallback, HWND.NULL, 0, 0, 10, 10, SWP.SWP_NOACTIVATE | SWP.SWP_NOZORDER);
        drop.DisplayChanged();
        RECT dropRect, fallbackRect;
        GetWindowRect((HWND)drop.Handle, &dropRect);
        GetWindowRect(fallback, &fallbackRect);
        Assert.Equal((dropRect.left, dropRect.top, dropRect.right, dropRect.bottom),
            (fallbackRect.left, fallbackRect.top, fallbackRect.right, fallbackRect.bottom));
        Assert.Equal(foreground, GetForegroundWindow());

        drop.Update(InkState.Idle);
        Assert.False(drop.ShowsFallback);
        Assert.False(Visible(fallback));
        Assert.Equal(foreground, GetForegroundWindow());
    }

    /// <summary>
    /// A Drop whose plain panel cannot be made says so from the start, and says both when the ink
    /// fails too: the shell shows it where it stays seen (the tray icon).
    /// </summary>
    [Fact]
    public void AFallbackThatCannotBeMadeIsSaidAtOnceAndWithTheInksFailure()
    {
        var ui = new UiThread();
        var loader = new InkPipelineLoader(() => new InkPipeline(InkAdapter.Warp, "not a shader"));
        Assert.True(loader.Wait().Permanent);
        var tries = 0;
        using var drop = new DropWindow(loader, new InkClock(ui.Post), () =>
        {
            tries++;
            throw new InkRendererException("couldn't make the Drop's fallback window (error 8)");
        });
        var problems = new List<string?>();
        drop.ProblemChanged += problems.Add;
        Assert.Equal(1, tries);
        Assert.StartsWith("couldn't compile the ink shader", drop.Problem, StringComparison.Ordinal);
        Assert.EndsWith("; no plain panel either: couldn't make the Drop's fallback window (error 8)", drop.Problem, StringComparison.Ordinal);

        drop.Update(InkState.Meeting);
        ui.Pump(0.2);
        Assert.False(drop.ShowsFallback);
        Assert.True(tries >= 2, "the fallback is tried again when it is needed");
        Assert.Empty(problems);
    }

    [Fact]
    public void AFallbackThatCannotBeMadeIsSaidEvenWhileTheInkIsFine()
    {
        var ui = new UiThread();
        var loader = new InkPipelineLoader(() => new InkPipeline(InkAdapter.Warp));
        Assert.Null(loader.Wait().Failure);
        using var drop = new DropWindow(loader, new InkClock(ui.Post),
            () => throw new InkRendererException("couldn't make the Drop's fallback window (error 8)"));
        Assert.Equal("no plain panel to fall back on: couldn't make the Drop's fallback window (error 8)", drop.Problem);
        loader.Outcome!.Pipeline!.Dispose();
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

    /// <summary>
    /// A held take: its live words drawn on one line with the head cut, the ink reading the live
    /// levels once per frame. Needs a compositor (the desktop); skipped over SSH.
    /// </summary>
    [Fact]
    public void LiveWordsDrawAndTheInkReadsTheLevels()
    {
        lock (TestPipeline.Lock)
        {
            Assert.Null(Loader.Wait().Failure);
            var ui = new UiThread();
            using var drop = new DropWindow(Loader, new InkClock(ui.Post));
            drop.Surface.AssumeReduceMotion = false;
            var reads = 0;
            drop.Surface.Levels = () =>
            {
                reads++;
                return new InkLevels(0.8, 0);
            };
            var words = string.Join(' ', Enumerable.Repeat("a synthetic sentence said while the key is held", 6));
            drop.Show(new DropText("Dictating \u00B7 Notepad", words, LiveWords: true), InkState.Dictating);
            Assert.SkipWhen(drop.Surface.Failure?.EndsWith(NoCompositor, StringComparison.Ordinal) == true,
                $"no compositor in this session: {drop.Surface.Failure}");
            ui.Pump(0.3);
            drop.Show(new DropText("Dictating \u00B7 Notepad", words + " and more", LiveWords: true), InkState.Dictating);
            ui.Pump(0.3);
            Assert.Null(drop.Surface.Failure);
            Assert.True(drop.Surface.FramesDrawn > 5, $"live frames: {drop.Surface.FramesDrawn}");
            Assert.True(reads > 5, $"level reads: {reads}");
            drop.Hide();
        }
    }

    /// <summary>
    /// The Drop's window name (what screen readers and any process read) is the title only: never
    /// the detail, which holds the live words. Runs over SSH: no frame is needed.
    /// </summary>
    [Fact]
    public unsafe void TheWindowNameNeverHoldsTheWords()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            using var drop = new DropWindow(Loader, new InkClock(ui.Post));
            drop.Show(new DropText("Dictating \u00B7 Notepad", "a synthetic secret line", LiveWords: true), InkState.Dictating);
            var buffer = stackalloc char[256];
            var length = GetWindowTextW((HWND)drop.Handle, buffer, 256);
            Assert.Equal("Inkwell: Dictating \u00B7 Notepad", new string(buffer, 0, length));
            drop.Hide();
        }
        Assert.Equal("Inkwell: Too short", new DropText("Too short", "Try again").WindowTitle);
    }

    /// <summary>
    /// What a screen reader reads for the Drop is the Mac's accessibility label: the title and the
    /// detail, live words included.
    /// </summary>
    [Fact]
    public void TheScreenReaderNameIsTheMacs()
    {
        var held = new DropText("Dictating \u00B7 Notepad", "a synthetic line", LiveWords: true);
        Assert.Equal("Inkwell: Dictating \u00B7 Notepad, a synthetic line", held.AccessibleName);
        Assert.Equal("Inkwell: Dictating \u00B7 Notepad", held.WindowTitle);
        Assert.Equal("Inkwell: \u25CF REC, Recording this meeting", DropText.For(InkState.Meeting).AccessibleName);
        Assert.Equal("Inkwell: Too short, Try again", new DropText("Too short", "Try again").AccessibleName);
    }

    /// <summary>
    /// Narrator reads the Drop through UI Automation: its name is the title and the detail, the
    /// live words too, in a polite live region announced once per change (never for the same text
    /// again), without the Drop taking focus. The window's title stays the state only, and the
    /// words go when the Drop hides. Runs over SSH: no frame is needed.
    /// </summary>
    [Fact]
    public void ScreenReadersReadTheLiveWordsAndTheTitleNeverHoldsThem()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            using var drop = new DropWindow(Loader, new InkClock(ui.Post));
            using var events = new LiveRegionEvents();
            var foreground = GetForegroundWindow();
            var held = new DropText("Dictating \u00B7 Notepad", "a synthetic line", LiveWords: true);
            drop.Show(held, InkState.Dictating);
            ui.Pump(0.1);
            var read = Uia.Read(drop.Handle, ui);
            Assert.Equal("Inkwell: Dictating \u00B7 Notepad, a synthetic line", read.Name);
            Assert.Equal(ScreenReaderName.Polite, read.LiveSetting);
            Assert.Equal("Inkwell: Dictating \u00B7 Notepad", WindowText(drop.Handle));
            Assert.Equal(1, events.For(drop.Handle));

            var grown = held with { Detail = "a synthetic line grows" };
            drop.Show(grown, InkState.Dictating);
            drop.Show(grown, InkState.Dictating);
            ui.Pump(0.1);
            Assert.Equal("Inkwell: Dictating \u00B7 Notepad, a synthetic line grows", Uia.Read(drop.Handle, ui).Name);
            Assert.Equal(2, events.For(drop.Handle));
            Assert.Equal("Inkwell: Dictating \u00B7 Notepad", WindowText(drop.Handle));
            Assert.Equal(foreground, GetForegroundWindow());
            Assert.NotEqual((HWND)drop.Handle, GetActiveWindow());

            // Hidden, the words go; shown again with the same lines, they are said again.
            drop.Hide();
            ui.Pump(0.1);
            Assert.DoesNotContain("synthetic", Uia.Read(drop.Handle, ui).Name ?? "", StringComparison.Ordinal);
            drop.Show(grown, InkState.Dictating);
            ui.Pump(0.1);
            Assert.Equal(3, events.For(drop.Handle));
            drop.Hide();
        }
    }

    /// <summary>
    /// The plain fallback, over the Drop, reads the same to a screen reader and has the same
    /// title; only the Drop's own window announces, so nothing is said twice.
    /// </summary>
    [Fact]
    public void ThePlainFallbackReadsTheSameAndOnlyTheDropAnnounces()
    {
        var ui = new UiThread();
        var loader = new InkPipelineLoader(() => new InkPipeline(InkAdapter.Warp, "not a shader"));
        Assert.True(loader.Wait().Permanent);
        using var drop = new DropWindow(loader, new InkClock(ui.Post));
        ui.Pump(0.2);
        using var events = new LiveRegionEvents();
        var held = new DropText("Dictating \u00B7 Notepad", "a synthetic line", LiveWords: true);
        drop.Show(held, InkState.Dictating);
        ui.Pump(0.2);
        Assert.True(drop.ShowsFallback);
        var fallback = drop.FallbackHandle;
        Assert.Equal(held.AccessibleName, Uia.Read(fallback, ui).Name);
        Assert.Equal("Inkwell: Dictating \u00B7 Notepad", WindowText(fallback));
        Assert.Equal(1, events.For(drop.Handle));
        Assert.Equal(0, events.For(fallback));

        drop.Hide();
        ui.Pump(0.1);
        Assert.DoesNotContain("synthetic", Uia.Read(fallback, ui).Name ?? "", StringComparison.Ordinal);
    }

    private static unsafe string WindowText(nint hwnd)
    {
        var buffer = stackalloc char[256];
        var length = GetWindowTextW((HWND)hwnd, buffer, 256);
        return new string(buffer, 0, length);
    }

    [Fact]
    public void TheWetWordsAreTheLastTwo()
    {
        Assert.Equal("said so", "we said so"[DropText.WetStart("we said so")..]);
        Assert.Equal("said so  ", "we said so  "[DropText.WetStart("we said so  ")..]);
        Assert.Equal(0, DropText.WetStart("hello"));
        Assert.Equal(0, DropText.WetStart("two words"));
        Assert.Equal(0, DropText.WetStart(""));
    }

    [Fact]
    public void LiveWordsAreCutFromTheHeadSoTheNewestShow()
    {
        // One unit per character.
        static double Width(string s) => s.Length;
        Assert.Equal("short words", DropText.HeadCut("short words", Width, 20));
        Assert.Equal("  trimmed".Trim(), DropText.HeadCut("  trimmed ", Width, 20));
        var cut = DropText.HeadCut("one two three four five six", Width, 12);
        Assert.Equal("\u2026five six", cut);
        Assert.True(Width(cut) <= 12);
        // The longest tail that fits: dropping one word fewer would not.
        Assert.True(Width("\u2026four five six") > 12);
        // A last word too long alone is left for the layout's own cut.
        Assert.Equal("\u2026enormousword", DropText.HeadCut("a enormousword", Width, 5));
        Assert.Equal("enormousword", DropText.HeadCut("enormousword", Width, 5));
    }

    [Fact]
    public void EachStateSaysWhatTheMacSays()
    {
        Assert.Equal(new DropText("Dictating", "Listening"), DropText.For(InkState.Dictating));
        Assert.Equal(new DropText("\u25CF REC", "Recording this meeting", DropTone.Recording), DropText.For(InkState.Meeting));
        Assert.Equal(new DropText("Blotting", "The final pass"), DropText.For(InkState.Blotting));
        Assert.Equal(new DropText("Far end silent", "Nothing is arriving from the call", DropTone.Alert), DropText.For(InkState.Problem));
    }
}
