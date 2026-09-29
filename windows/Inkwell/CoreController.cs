// Starts and stops the core, and brings its events to the UI thread.
//
//   core event thread --Push--> EventRelay --one hop per batch--> UI thread: the status
//
// The shell only sends commands and renders state (architecture rule 1). Nothing here polls.
using Inkwell.Core;
using Inkwell.Core.Events;
using Microsoft.UI.Dispatching;

namespace Inkwell;

public sealed class CoreController(DispatcherQueue ui, Action<CoreStatus> show)
{
    private InkSession? session;
    private CoreStatus status = new(CoreStatusKind.Starting);

    /// <summary>UI thread. Starts the core with the library in the data directory; a failure is shown.</summary>
    public void Start()
    {
        if (session is not null)
        {
            return;
        }
        status = new CoreStatus(CoreStatusKind.Starting);
        show(status);
        var relay = new EventRelay(work => ui.TryEnqueue(() => work()), Received);
        try
        {
            var data = DataLocation.DataDirectory();
            Directory.CreateDirectory(data);
            session = InkSession.Start(new InkConfig(data, DataLocation.ModelsDirectory()), e => relay.Push(e));
        }
        catch (Exception e) when (e is InkStatusException or IOException or UnauthorizedAccessException or DllNotFoundException or EntryPointNotFoundException)
        {
            // The message names a path or a core status, never anything the user said.
            status = new CoreStatus(CoreStatusKind.Failed, e.Message);
            show(status);
        }
    }

    /// <summary>UI thread: a batch of the core's events.</summary>
    private void Received(IReadOnlyList<InkEvent> batch)
    {
        foreach (var e in batch)
        {
            var next = status.Next(e, Log);
            if (next != status)
            {
                status = next;
                show(status);
            }
        }
    }

    /// <summary>What no screen shows, by name only (CoreStatus.Next never passes an event's words).</summary>
    private static void Log(string line) => System.Diagnostics.Trace.WriteLine($"Inkwell: {line}");

    /// <summary>
    /// UI thread. Stops the core off the UI thread (it waits for its workers and unloads every
    /// model), then calls <paramref name="done"/> on the UI thread. When done runs, no event arrives.
    /// </summary>
    public void Stop(Action done)
    {
        var stopping = session;
        session = null;
        if (stopping is null)
        {
            done();
            return;
        }
        Task.Run(() =>
        {
            stopping.Shutdown();
            ui.TryEnqueue(() => done());
        });
    }
}

/// <summary>
/// Where the app keeps its data: %LOCALAPPDATA%\Inkwell (the library, recordings and models).
/// INK_DATA_DIR and INK_MODELS_DIR move the library and the models elsewhere, as on the Mac.
/// </summary>
internal static class DataLocation
{
    public static string DataDirectory() =>
        Override("INK_DATA_DIR")
        ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Inkwell");

    public static string? ModelsDirectory() => Override("INK_MODELS_DIR");

    private static string? Override(string name)
    {
        var value = Environment.GetEnvironmentVariable(name);
        if (string.IsNullOrEmpty(value))
        {
            return null;
        }
        if (!Path.IsPathFullyQualified(value))
        {
            throw new IOException($"{name} must be an absolute path");
        }
        return value;
    }
}
