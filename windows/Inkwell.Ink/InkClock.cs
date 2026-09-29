// One frame clock for every live ink: the Mac's InkClock (one display link) on Windows. With the
// Drop and a window's ink both live, one tick per compositor frame steps and draws both on the UI
// thread. Each surface's schedule decides whether it takes part: a surface joins when its ink goes
// live on screen and leaves when it settles, hides or cannot draw.
//
// The ticks come from a thread that waits on the compositor's clock
// (DCompositionWaitForCompositorClock: once per frame DWM composes) and posts one tick at a time to
// the UI thread, never queueing a second before the first has run. Like the Mac's display link
// (pinned to 60 fps), it ticks at most about 60 times a second: on a faster display it skips
// compositor frames (120 Hz: every other one). The thread exists only while a surface is on the
// clock (architecture rule 9: nothing ticks while idle); the last surface to leave ends it within
// one frame.
using System.Diagnostics;
using TerraFX.Interop.DirectX;

namespace Inkwell.Ink;

/// <summary>The clock the live inks share. Everything but the tick thread runs on the UI thread.</summary>
public sealed class InkClock
{
    private readonly Action<Action> post;
    private readonly Func<bool> waitForFrame;
    private readonly List<InkSurface> clients = [];
    private int generation;
    private int pending;
    private int threads;

    /// <summary>
    /// A clock that runs its ticks through <paramref name="post"/> (onto the UI thread).
    /// <paramref name="waitForFrame"/> blocks until the next frame and says whether one came (tests
    /// replace it); by default, the compositor's clock.
    /// </summary>
    public InkClock(Action<Action> post, Func<bool>? waitForFrame = null)
    {
        this.post = post;
        this.waitForFrame = waitForFrame ?? CompositorFrame;
    }

    /// <summary>Hands <paramref name="work"/> to the UI thread.</summary>
    public void Post(Action work) => post(work);

    /// <summary>Surfaces on the clock now.</summary>
    public int ClientCount => clients.Count;

    /// <summary>Tick threads alive now: 1 while a surface is on the clock, 0 shortly after the last leaves.</summary>
    public int ThreadCount => Volatile.Read(ref threads);

    internal bool Contains(InkSurface surface) => clients.Contains(surface);

    /// <summary>Puts a surface on the clock; the first starts the tick thread.</summary>
    internal void Add(InkSurface surface)
    {
        if (clients.Contains(surface))
        {
            return;
        }
        clients.Add(surface);
        if (clients.Count == 1)
        {
            Start();
        }
    }

    /// <summary>Takes a surface off the clock; the last to leave stops the tick thread.</summary>
    internal void Remove(InkSurface surface)
    {
        if (clients.Remove(surface) && clients.Count == 0)
        {
            // The thread sees the new generation after its current wait and ends.
            Interlocked.Increment(ref generation);
        }
    }

    /// <summary>One frame: every surface on the clock steps and draws, in the order they joined. UI thread.</summary>
    internal void Tick(double now)
    {
        // A surface may leave during its own tick; the others still get theirs.
        foreach (var surface in clients.ToArray())
        {
            if (clients.Contains(surface))
            {
                surface.ClockTicked(now);
            }
        }
    }

    /// <summary>Seconds on a monotonic clock.</summary>
    internal static double Now() => Stopwatch.GetTimestamp() / (double)Stopwatch.Frequency;

    private void Start()
    {
        var mine = Interlocked.Increment(ref generation);
        Interlocked.Increment(ref threads);
        var thread = new Thread(() => Run(mine)) { IsBackground = true, Name = "ink-clock" };
        thread.Start();
    }

    /// <summary>The shortest gap between ticks: a 60 Hz frame less 3 ms of slack for the compositor's jitter.</summary>
    private const double MinInterval = 1.0 / 60 - 0.003;

    private void Run(int mine)
    {
        var last = double.NegativeInfinity;
        try
        {
            while (Volatile.Read(ref generation) == mine)
            {
                if (!waitForFrame() || Volatile.Read(ref generation) != mine)
                {
                    continue;
                }
                var now = Now();
                if (now - last < MinInterval)
                {
                    continue;
                }
                last = now;
                // One tick in flight at most: a slow frame skips ticks rather than queueing them.
                if (Interlocked.Exchange(ref pending, 1) == 0)
                {
                    post(() =>
                    {
                        Volatile.Write(ref pending, 0);
                        if (clients.Count > 0)
                        {
                            Tick(Now());
                        }
                    });
                }
            }
        }
        finally
        {
            Interlocked.Decrement(ref threads);
        }
    }

    private static bool fallbackLogged;

    /// <summary>Blocks until DWM's next frame (at most 100 ms). False on a timeout.</summary>
    private static unsafe bool CompositorFrame()
    {
        const uint waitTimeout = 0x102; // WAIT_TIMEOUT
        var result = DirectX.DCompositionWaitForCompositorClock(0, null, 100);
        if (result == 0)
        {
            return true;
        }
        if (result == waitTimeout)
        {
            return false;
        }
        // No compositor clock (a session without DWM, such as SSH): sleep instead, and say so once.
        // Nothing shows in such a session; the sleep keeps the schedule and its tests honest.
        if (!fallbackLogged)
        {
            fallbackLogged = true;
            InkLog.Write($"couldn't wait on the compositor clock (0x{result:X8}, error {System.Runtime.InteropServices.Marshal.GetLastSystemError()}); pacing the ink with a 16 ms sleep instead");
        }
        Thread.Sleep(16);
        return true;
    }
}
