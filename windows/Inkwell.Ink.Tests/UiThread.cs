// A stand-in for the app's UI thread in tests: work posted by the clock (or the pipeline loader)
// queues here and runs when the test pumps, on the test's own thread, which also pumps Win32
// messages for any window it made.
using System.Collections.Concurrent;
using System.Diagnostics;
using TerraFX.Interop.Windows;
using static TerraFX.Interop.Windows.Windows;

namespace Inkwell.Ink.Tests;

internal sealed class UiThread
{
    private readonly ConcurrentQueue<Action> work = new();

    public void Post(Action action) => work.Enqueue(action);

    /// <summary>Runs posted work and window messages for <paramref name="seconds"/>.</summary>
    public unsafe void Pump(double seconds)
    {
        var until = Stopwatch.GetTimestamp() + (long)(seconds * Stopwatch.Frequency);
        MSG msg;
        while (Stopwatch.GetTimestamp() < until)
        {
            while (PeekMessageW(&msg, HWND.NULL, 0, 0, PM.PM_REMOVE))
            {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // Only what was queued before this pass: a clock whose frames outlast its interval
            // (a Present waiting on a compositor fallen behind) queues the next tick during each
            // one, and draining until empty never came back to the deadline.
            for (var queued = work.Count; queued > 0 && work.TryDequeue(out var action); queued--)
            {
                action();
            }
            Thread.Sleep(1);
        }
    }
}
