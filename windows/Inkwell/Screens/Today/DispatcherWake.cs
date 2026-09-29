// A one-shot wake on the UI thread, for the models that wait once (Up next's rollover at an
// event's start): a DispatcherQueueTimer that fires once and stops, never one that repeats.
// Disposing the handle cancels it.
using Inkwell.Core.Screens;
using Microsoft.UI.Dispatching;

namespace Inkwell.Screens;

public sealed class DispatcherWake(DispatcherQueue queue) : IWakeScheduler
{
    public IDisposable After(TimeSpan delay, Action wake)
    {
        ArgumentNullException.ThrowIfNull(wake);
        var timer = queue.CreateTimer();
        timer.IsRepeating = false;
        timer.Interval = delay > TimeSpan.Zero ? delay : TimeSpan.Zero;
        timer.Tick += (t, _) =>
        {
            t.Stop();
            wake();
        };
        timer.Start();
        return new Handle(timer);
    }

    private sealed class Handle(DispatcherQueueTimer timer) : IDisposable
    {
        public void Dispose() => timer.Stop();
    }
}
