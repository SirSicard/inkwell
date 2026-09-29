// Compiles the ink's pipeline off the UI thread and hands it to everything that uses it: the Mac's
// InkPipelineLoader. The app starts the compile at launch, next to the core's start, so no surface
// ever compiles on the UI thread: a surface made before the compile finishes starts drawing when
// the pipeline arrives.
//
// Unlike the Mac's, a pipeline here can be lost: a GPU reset (TDR), a driver update or a DWM
// restart kills the Direct3D device under it. A surface whose frame fails asks for a new one
// (Recreate); every subscriber gets the new outcome, and the lost pipeline is released after them.
// A shader that does not compile is permanent: asking again gives the same answer.
namespace Inkwell.Ink;

/// <summary>A compile's outcome: a pipeline, or why there is none (and whether asking again can help).</summary>
public sealed record InkPipelineOutcome(InkPipeline? Pipeline, string? Failure, bool Permanent = false);

/// <summary>Compiles the pipeline off the UI thread, and again after the device is lost.</summary>
public sealed class InkPipelineLoader(Func<InkPipeline> make)
{
    private readonly Lock gate = new();
    private readonly List<Action<InkPipelineOutcome>> subscribers = [];
    private bool compiling;
    private InkPipelineOutcome? outcome;

    /// <summary>The process's loader: the embedded shader on the GPU (WARP when there is none).</summary>
    public static InkPipelineLoader Shared { get; } = new(() => new InkPipeline());

    /// <summary>How long the last compile took, once it has finished.</summary>
    public TimeSpan? Duration { get; private set; }

    /// <summary>Compiles made so far (tests: a recreate happened).</summary>
    public int Compiles { get; private set; }

    /// <summary>Starts the compile on the thread pool, the first time only. Any thread; never waits.</summary>
    public void Warm()
    {
        lock (gate)
        {
            if (compiling || outcome is not null)
            {
                return;
            }
            compiling = true;
        }
        ThreadPool.UnsafeQueueUserWorkItem(_ => Compile(null, null), null);
    }

    /// <summary>
    /// The pipeline <paramref name="lost"/> failed (or the last compile failed and may succeed
    /// now): compile a new one, unless that already happened or is happening. The lost pipeline is
    /// released through <paramref name="post"/> (the UI thread), after every subscriber has had the
    /// new outcome. A permanent failure is not retried.
    /// </summary>
    public void Recreate(InkPipeline? lost, Action<Action> post)
    {
        lock (gate)
        {
            // Nothing to do while a compile runs, after a permanent failure, or when the pipeline
            // was already replaced (another surface asked first; the new one reaches every
            // subscriber). A failed recreate may be asked again.
            if (compiling || outcome is null || outcome.Permanent || (outcome.Pipeline is not null && outcome.Pipeline != lost))
            {
                return;
            }
            compiling = true;
        }
        ThreadPool.UnsafeQueueUserWorkItem(_ => Compile(lost, post), null);
    }

    private void Compile(InkPipeline? lost, Action<Action>? post)
    {
        var began = System.Diagnostics.Stopwatch.GetTimestamp();
        InkPipelineOutcome result;
        try
        {
            result = new InkPipelineOutcome(make(), null);
        }
        catch (InkRendererException e)
        {
            result = new InkPipelineOutcome(null, e.Message, e.Permanent);
        }
        catch (Exception e)
        {
            // Anything else (a missing system DLL, an unreadable resource) must not end the app on
            // a pool thread: it becomes the named failure every subscriber gets.
            result = new InkPipelineOutcome(null, $"couldn't start the ink: {e.GetType().Name}: {e.Message}");
        }
        Action<InkPipelineOutcome>[] ready;
        lock (gate)
        {
            outcome = result;
            compiling = false;
            Compiles++;
            Duration = System.Diagnostics.Stopwatch.GetElapsedTime(began);
            ready = [.. subscribers];
        }
        if (result.Failure is not null)
        {
            InkLog.Write(result.Failure);
        }
        foreach (var subscriber in ready)
        {
            subscriber(result);
        }
        if (lost is not null && post is not null)
        {
            // Queued after the subscribers' own hops to the UI thread: none still draws with it.
            post(lost.Dispose);
        }
    }

    /// <summary>The latest outcome, else null. Never waits.</summary>
    public InkPipelineOutcome? Outcome
    {
        get
        {
            lock (gate)
            {
                return outcome;
            }
        }
    }

    /// <summary>
    /// Calls <paramref name="body"/> on the UI thread (through <paramref name="post"/>) with the
    /// current outcome, if there is one, and with every outcome after it (a pipeline made again
    /// after a lost device). Starts the compile if nothing has. Dispose the result to stop.
    /// </summary>
    public IDisposable Subscribe(Action<Action> post, Action<InkPipelineOutcome> body)
    {
        ArgumentNullException.ThrowIfNull(post);
        Action<InkPipelineOutcome> subscriber = result => post(() => body(result));
        InkPipelineOutcome? known;
        lock (gate)
        {
            subscribers.Add(subscriber);
            known = outcome;
        }
        Warm();
        if (known is not null)
        {
            subscriber(known);
        }
        return new Subscription(this, subscriber);
    }

    private sealed class Subscription(InkPipelineLoader loader, Action<InkPipelineOutcome> subscriber) : IDisposable
    {
        public void Dispose()
        {
            lock (loader.gate)
            {
                loader.subscribers.Remove(subscriber);
            }
        }
    }

    /// <summary>Calls <paramref name="body"/> once, on the UI thread, with the first outcome.</summary>
    public void WhenReady(Action<Action> post, Action<InkPipelineOutcome> body)
    {
        IDisposable? subscription = null;
        var done = false;
        subscription = Subscribe(post, result =>
        {
            if (done)
            {
                return;
            }
            done = true;
            subscription?.Dispose();
            body(result);
        });
    }

    /// <summary>Waits for the first outcome, starting the compile if nothing has. Tests and offline renders only.</summary>
    public InkPipelineOutcome Wait()
    {
        using var ready = new ManualResetEventSlim();
        InkPipelineOutcome? result = null;
        using (Subscribe(work => work(), r =>
        {
            result ??= r;
            ready.Set();
        }))
        {
            ready.Wait();
        }
        return result!;
    }
}
