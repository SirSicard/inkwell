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
        Assert.Equal("Inkwell: Too short", new DropText("Too short", "Try again").AccessibleName);
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

    /// <summary>The consent offer (S3.5b): two buttons, which widen the panel as on the Mac.</summary>
    private static readonly DropText Offer = new("Example Call opened the microphone", "Recording keeps both sides on this PC. Tell the others you are recording.")
    {
        Buttons = new DropButtons("Record this call", "Not this one"),
    };

    [Fact]
    public void TheOffersButtonsLieInARowInItsWiderPanel()
    {
        Assert.Equal((DropLayout.WidthWithButtons, DropLayout.HeightWithButtons), DropLayout.Size(Offer));
        Assert.Equal((DropLayout.Width, DropLayout.Height), DropLayout.Size(DropText.For(InkState.Meeting)));
        var (l0, t0, r0, b0) = DropLayout.Button(0);
        var (l1, t1, r1, b1) = DropLayout.Button(1);
        Assert.True(l0 >= DropLayout.TextLeft && r0 < l1 && r1 <= DropLayout.WidthWithButtons - DropLayout.TextRight, "inside the panel, beside the ink");
        Assert.True(t0 == t1 && b0 == b1 && b1 < DropLayout.HeightWithButtons, "one row, above the bottom edge");
        Assert.Equal(0, DropLayout.ButtonAt(Offer.Buttons, (l0 + r0) / 2, (t0 + b0) / 2));
        Assert.Equal(1, DropLayout.ButtonAt(Offer.Buttons, (l1 + r1) / 2, (t1 + b1) / 2));
        Assert.Null(DropLayout.ButtonAt(Offer.Buttons, (r0 + l1) / 2, (t0 + b0) / 2)); // the gap
        Assert.Null(DropLayout.ButtonAt(Offer.Buttons, DropLayout.InkWidth / 2, (t0 + b0) / 2)); // the ink
        Assert.Null(DropLayout.ButtonAt(null, (l0 + r0) / 2, (t0 + b0) / 2));
        Assert.Null(DropLayout.ButtonAt(new DropButtons("One"), (l1 + r1) / 2, (t1 + b1) / 2));
        Assert.Equal(2, Offer.Buttons!.Count);
        Assert.Equal("Not this one", Offer.Buttons[1]);
    }

    /// <summary>
    /// A click on a button (down and up on it) says which, and never activates the Drop; the panel
    /// takes its wider size for the offer and its own size back after. Runs over SSH: messages, no frame.
    /// </summary>
    [Fact]
    public unsafe void AClickOnAButtonSaysWhichAndNeverActivatesTheDrop()
    {
        lock (TestPipeline.Lock)
        {
            var ui = new UiThread();
            using var drop = new DropWindow(Loader, new InkClock(ui.Post));
            var hwnd = (HWND)drop.Handle;
            var clicked = new List<int>();
            drop.ButtonClicked += clicked.Add;
            var foreground = GetForegroundWindow();

            drop.Show(DropText.For(InkState.Meeting), InkState.Meeting);
            var small = Rect(hwnd);
            drop.Show(Offer, InkState.Idle);
            ui.Pump(0.1);
            var wide = Rect(hwnd);
            var scale = (wide.right - wide.left) / DropLayout.WidthWithButtons;
            Assert.True(wide.right - wide.left > small.right - small.left, "wider");
            Assert.True(wide.bottom - wide.top > small.bottom - small.top, "taller");
            Assert.Equal(small.bottom, wide.bottom); // the same bottom margin

            Click(hwnd, 1, 1, scale);
            Click(hwnd, 0, 0, scale);
            Assert.Equal([1, 0], clicked);
            // Down on one, up on the other: no click.
            Click(hwnd, 0, 1, scale);
            Assert.Equal([1, 0], clicked);
            Assert.Equal(foreground, GetForegroundWindow());
            Assert.NotEqual(hwnd, GetActiveWindow());

            // Answered: the panel goes back to its size, and where the buttons were is nothing.
            drop.Show(DropText.For(InkState.Meeting), InkState.Meeting);
            var back = Rect(hwnd);
            Assert.Equal((small.right - small.left, small.bottom - small.top), (back.right - back.left, back.bottom - back.top));
            Click(hwnd, 0, 0, scale);
            Assert.Equal([1, 0], clicked);
            drop.Hide();
        }
    }

    /// <summary>When the ink cannot draw, the plain panel shows the offer with its buttons, and they answer too.</summary>
    [Fact]
    public unsafe void ThePlainFallbackShowsTheOfferAndItsButtonsAnswer()
    {
        var ui = new UiThread();
        var loader = new InkPipelineLoader(() => new InkPipeline(InkAdapter.Warp, "not a shader"));
        Assert.True(loader.Wait().Permanent);
        using var drop = new DropWindow(loader, new InkClock(ui.Post));
        var clicked = new List<int>();
        drop.ButtonClicked += clicked.Add;
        ui.Pump(0.2);
        var foreground = GetForegroundWindow();

        drop.Show(Offer, InkState.Idle);
        ui.Pump(0.2);
        Assert.True(drop.ShowsFallback, "an offer is never invisible");
        var fallback = (HWND)drop.FallbackHandle;
        var rect = Rect(fallback);
        var scale = (rect.right - rect.left) / DropLayout.WidthWithButtons;
        Click(fallback, 0, 0, scale);
        Assert.Equal([0], clicked);
        Assert.Equal(foreground, GetForegroundWindow());
        Assert.NotEqual(fallback, GetActiveWindow());
        drop.Hide();
    }

    /// <summary>
    /// Review (S3.5b): the plain fallback draws the offer's detail on up to two lines above the
    /// buttons, so the consent sentence shows whole (one line cut it after about 40 characters);
    /// a panel without buttons keeps its one line. Measured with the fallback's own font.
    /// </summary>
    [Theory]
    [InlineData(1.0)]
    [InlineData(1.5)]
    public unsafe void ThePlainFallbackShowsTheWholeConsentSentence(double scale)
    {
        int S(double dips) => (int)Math.Round(dips * scale);
        var client = new RECT { right = S(DropLayout.WidthWithButtons), bottom = S(DropLayout.HeightWithButtons) };
        var (title, detail, format) = DropFallback.Lines(buttons: true, client, scale);
        Assert.True((format & DT.DT_WORDBREAK) != 0 && (format & DT.DT_SINGLELINE) == 0, "wraps");
        Assert.True(title.bottom <= detail.top, "the title above the detail");
        Assert.True(detail.bottom <= S(DropLayout.Button(0).Top), "above the buttons");

        var dc = CreateCompatibleDC(HDC.NULL);
        var font = DropFallback.Font(S(DropLayout.DetailSize), FW.FW_NORMAL);
        var before = SelectObject(dc, (HGDIOBJ)font.Value);
        try
        {
            TEXTMETRICW metrics;
            Assert.True(GetTextMetricsW(dc, &metrics) != 0);
            var needed = detail;
            var words = Offer.Detail;
            fixed (char* w = words)
            {
                // The height the whole sentence takes, wrapped at the detail's width.
                Assert.True(DrawTextW(dc, w, words.Length, &needed, DT.DT_CALCRECT | DT.DT_WORDBREAK | DT.DT_NOPREFIX | DT.DT_LEFT | DT.DT_TOP) > 0);
            }
            var height = needed.bottom - needed.top;
            Assert.True(height > metrics.tmHeight, $"more than one line ({height} px, a line is {metrics.tmHeight} px)");
            Assert.True(height <= detail.bottom - detail.top, $"{height} px fits the detail's {detail.bottom - detail.top} px");
        }
        finally
        {
            SelectObject(dc, before);
            DeleteObject((HGDIOBJ)font.Value);
            DeleteDC(dc);
        }

        var plain = new RECT { right = S(DropLayout.Width), bottom = S(DropLayout.Height) };
        Assert.True((DropFallback.Lines(buttons: false, plain, scale).DetailFormat & DT.DT_SINGLELINE) != 0);
    }

    private static unsafe RECT Rect(HWND hwnd)
    {
        RECT rect;
        GetWindowRect(hwnd, &rect);
        return rect;
    }

    /// <summary>
    /// A left button pressed on button <paramref name="down"/> and let go on <paramref name="up"/>, in
    /// client pixels. First the question Windows asks a window before a click activates it
    /// (WM_MOUSEACTIVATE, sent here as Windows would: the top-level window, the client area, the
    /// button's message); the window must answer "don't activate", or the click would take focus
    /// from the call.
    /// </summary>
    private static unsafe void Click(HWND hwnd, int down, int up, double scale)
    {
        var asked = SendMessageW(hwnd, WM.WM_MOUSEACTIVATE, (WPARAM)(nuint)(nint)hwnd.Value,
            (LPARAM)(nint)((WM.WM_LBUTTONDOWN << 16) | HTCLIENT));
        Assert.Equal((nint)MA.MA_NOACTIVATE, (nint)asked);
        foreach (var (msg, button) in new[] { (WM.WM_LBUTTONDOWN, down), (WM.WM_LBUTTONUP, up) })
        {
            var (l, t, r, b) = DropLayout.Button(button);
            var x = (int)Math.Round((l + r) / 2 * scale);
            var y = (int)Math.Round((t + b) / 2 * scale);
            // MK_LBUTTON while the button is down; the point packed as MAKELPARAM does.
            SendMessageW(hwnd, (uint)msg, (WPARAM)(nuint)(msg == WM.WM_LBUTTONDOWN ? 1 : 0), (LPARAM)(nint)((y << 16) | (x & 0xFFFF)));
        }
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
