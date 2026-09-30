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
            while (work.TryDequeue(out var action))
            {
                action();
            }
            Thread.Sleep(1);
        }
    }
}
