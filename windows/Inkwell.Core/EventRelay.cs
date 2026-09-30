// The hop from the core's event thread to the UI thread (inkwell.h, THREADS 1: hop to the main
// thread and return promptly; never wait on it).
//
// Events are queued under a lock and handed over in batches: the first event after a hand-over
// posts one work item to the UI thread, and every event that arrives before that item runs rides
// along. However fast the core talks, the UI thread wakes at most once per turn of its loop, and
// nothing wakes it at all while the core is quiet. No timer is involved (architecture rule 9).
using Inkwell.Core.Events;

namespace Inkwell.Core;

/// <summary>Carries events from the core's event thread to the UI thread, in order, in batches.</summary>
/// <param name="post">
/// Queues work on the UI thread and returns at once (the WinUI shell passes
/// <c>DispatcherQueue.TryEnqueue</c>); false when the UI thread is gone.
/// </param>
/// <param name="deliver">Receives each batch on the UI thread, oldest event first; never empty.</param>
public sealed class EventRelay(Func<Action, bool> post, Action<IReadOnlyList<InkEvent>> deliver)
{
    private readonly Lock gate = new();
    private List<InkEvent> waiting = [];
    private bool handOverQueued;
    private int handOvers;

    /// <summary>
    /// Event thread (any thread). Queues <paramref name="evt"/> for the UI thread and returns at
    /// once. False when the UI thread refused the hand-over (it is shutting down): the events
    /// waiting for it are dropped, and the next push tries again.
    /// </summary>
    public bool Push(InkEvent evt)
    {
        lock (gate)
        {
            waiting.Add(evt);
            if (handOverQueued)
            {
                return true;
            }
            handOverQueued = true;
            handOvers++;
        }
        if (post(HandOver))
        {
            return true;
        }
        lock (gate)
        {
            handOverQueued = false;
            waiting.Clear();
        }
        return false;
    }

    /// <summary>UI-thread work items queued so far: how often the core woke the UI thread.</summary>
    public int HandOvers
    {
        get
        {
            lock (gate)
            {
                return handOvers;
            }
        }
    }

    private void HandOver()
    {
        List<InkEvent> batch;
        lock (gate)
        {
            // Cleared together with taking the events: an event pushed after this point queues a
            // hand-over of its own, which runs after this one on the UI thread's queue.
            handOverQueued = false;
            batch = waiting;
            waiting = [];
        }
        if (batch.Count > 0)
        {
            deliver(batch);
        }
    }
}
