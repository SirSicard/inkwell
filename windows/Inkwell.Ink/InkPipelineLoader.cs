// Compiles the ink's pipeline once, on the thread pool, and hands it to everything that waits for
// it: the Mac's InkPipelineLoader. The app starts the compile at launch, next to the core's start,
// so no surface ever compiles on the UI thread: a surface made before the compile finishes shows
// paper and starts drawing when the pipeline arrives. A failure is kept: a machine where
// Direct3D 11 cannot start, or a build without the shader, does not retry.
namespace Inkwell.Ink;

/// <summary>The compile's outcome: a pipeline, or why there is none.</summary>
public sealed record InkPipelineOutcome(InkPipeline? Pipeline, string? Failure);

/// <summary>Compiles the pipeline once, off the UI thread.</summary>
public sealed class InkPipelineLoader(Func<InkPipeline> make)
{
    private readonly Lock gate = new();
    private readonly List<Action<InkPipelineOutcome>> waiters = [];
    private bool started;
    private InkPipelineOutcome? outcome;

    /// <summary>The process's loader: the embedded shader on the GPU (WARP when there is none).</summary>
    public static InkPipelineLoader Shared { get; } = new(() => new InkPipeline());

    /// <summary>How long the compile took, once it has finished.</summary>
    public TimeSpan? Duration { get; private set; }

    /// <summary>Starts the compile on the thread pool, the first time only. Any thread; never waits.</summary>
    public void Warm()
    {
        lock (gate)
        {
            if (started)
            {
                return;
            }
            started = true;
        }
        ThreadPool.UnsafeQueueUserWorkItem(_ => Compile(), null);
    }

    private void Compile()
    {
        var began = System.Diagnostics.Stopwatch.GetTimestamp();
        InkPipelineOutcome result;
        try
        {
            result = new InkPipelineOutcome(make(), null);
        }
        catch (InkRendererException e)
        {
            result = new InkPipelineOutcome(null, e.Message);
        }
        List<Action<InkPipelineOutcome>> ready;
        lock (gate)
        {
            outcome = result;
            Duration = System.Diagnostics.Stopwatch.GetElapsedTime(began);
            ready = [.. waiters];
            waiters.Clear();
        }
        if (result.Failure is not null)
        {
            InkLog.Write(result.Failure);
        }
        foreach (var waiter in ready)
        {
            waiter(result);
        }
    }

    /// <summary>The outcome once the compile has finished, else null. Never waits.</summary>
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
    /// outcome, on a later turn even when it is already known. Starts the compile if nothing has.
    /// </summary>
    public void WhenReady(Action<Action> post, Action<InkPipelineOutcome> body)
    {
        ArgumentNullException.ThrowIfNull(post);
        Warm();
        Subscribe(result => post(() => body(result)));
    }

    /// <summary>Waits for the outcome, starting the compile if nothing has. Tests and offline renders only.</summary>
    public InkPipelineOutcome Wait()
    {
        Warm();
        using var done = new ManualResetEventSlim();
        InkPipelineOutcome? result = null;
        Subscribe(r =>
        {
            result = r;
            done.Set();
        });
        done.Wait();
        return result!;
    }

    private void Subscribe(Action<InkPipelineOutcome> waiter)
    {
        InkPipelineOutcome? ready;
        lock (gate)
        {
            ready = outcome;
            if (ready is null)
            {
                waiters.Add(waiter);
            }
        }
        if (ready is not null)
        {
            waiter(ready);
        }
    }
}
