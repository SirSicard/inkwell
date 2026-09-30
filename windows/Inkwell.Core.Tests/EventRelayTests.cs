// The hop to the UI thread: one hand-over per batch, in order, and nothing lost or repeated.
using Inkwell.Core.Events;
using Xunit;

namespace Inkwell.Core.Tests;

public class EventRelayTests
{
    private static UnknownEvent Event(string type) => new UnknownEvent { Type = type };

    [Fact]
    public void EventsBeforeTheHandOverRunRideAlongInOrder()
    {
        var ui = new Queue<Action>();
        var batches = new List<IReadOnlyList<InkEvent>>();
        var relay = new EventRelay(work => { ui.Enqueue(work); return true; }, batches.Add);

        Assert.True(relay.Push(Event("a")));
        Assert.True(relay.Push(Event("b")));
        Assert.True(relay.Push(Event("c")));
        Assert.Single(ui);
        ui.Dequeue()();
        Assert.True(relay.Push(Event("d")));
        Assert.Single(ui);
        ui.Dequeue()();

        Assert.Equal(2, relay.HandOvers);
        Assert.Equal([["a", "b", "c"], ["d"]], batches.Select(b => b.Select(e => e.Type).ToArray()));
    }

    [Fact]
    public void ARefusedHandOverDropsWhatWaitedAndTheNextPushTriesAgain()
    {
        var accept = false;
        var ui = new Queue<Action>();
        var batches = new List<IReadOnlyList<InkEvent>>();
        var relay = new EventRelay(work =>
        {
            if (accept)
            {
                ui.Enqueue(work);
            }
            return accept;
        }, batches.Add);

        Assert.False(relay.Push(Event("lost")));
        accept = true;
        Assert.True(relay.Push(Event("kept")));
        ui.Dequeue()();
        Assert.Equal(["kept"], batches.Single().Select(e => e.Type));
    }

    [Fact]
    public async Task PushesFromAnotherThreadAllArriveOnce()
    {
        var ui = new System.Collections.Concurrent.BlockingCollection<Action>();
        var seen = new List<string>();
        var relay = new EventRelay(work => { ui.Add(work); return true; }, batch =>
        {
            foreach (var e in batch)
            {
                seen.Add(e.Type);
            }
        });
        var pusher = Task.Run(() =>
        {
            for (var i = 0; i < 1000; i++)
            {
                relay.Push(Event(i.ToString(System.Globalization.CultureInfo.InvariantCulture)));
            }
        }, TestContext.Current.CancellationToken);
        // This thread plays the UI thread until every event has arrived.
        while (seen.Count < 1000)
        {
            Assert.True(ui.TryTake(out var work, TimeSpan.FromSeconds(10)), $"stalled at {seen.Count}");
            work();
        }
        await pusher;
        Assert.Equal(Enumerable.Range(0, 1000).Select(i => i.ToString(System.Globalization.CultureInfo.InvariantCulture)), seen);
        Assert.True(relay.HandOvers <= 1000);
    }
}
