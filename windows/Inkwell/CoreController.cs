// Starts and stops the core, and brings its events to the UI thread.
//
//   core event thread --Push--> EventRelay --one hop per batch--> UI thread: the store, then the screens
//
// The shell only sends commands and renders state (architecture rule 1). Nothing here polls.
using Inkwell.Core;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Dispatching;

namespace Inkwell;

public sealed class CoreController(DispatcherQueue ui)
{
    private InkSession? session;

    /// <summary>Everything the screens show about the core.</summary>
    public CoreStore Store { get; } = new();

    /// <summary>Sees every batch after the store (the screens' models), when set.</summary>
    public Action<IReadOnlyList<InkEvent>>? Observer { get; set; }

    /// <summary>Where commands that went nowhere are logged: by name only.</summary>
    public ScreenLog CommandLog { get; init; } = ScreenLog.System;

    /// <summary>UI thread. Starts the core with the library in the data directory; a failure is shown.</summary>
    public void Start()
    {
        if (session is not null)
        {
            return;
        }
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
            Store.StartFailed(e.Message);
        }
    }

    /// <summary>
    /// UI thread. Queues one of the screens' commands; its outcome arrives as events. A command
    /// with no core to take it, or one the core refuses to queue, never ran: logged by name.
    /// </summary>
    public void Send(CoreCommand command)
    {
        ArgumentNullException.ThrowIfNull(command);
        if (session is null)
        {
            CommandLog.Write($"no core is running: a {command.Name} command was not sent");
            NotSent(command, "the core is not running");
            return;
        }
        try
        {
            session.Command(command.Json);
        }
        catch (InkStatusException e)
        {
            CommandLog.Write($"the core refused a {command.Name} command: {e.Message}");
            NotSent(command, e.Message);
        }
    }

    /// <summary>
    /// A command that never reached the core fails as if the core had failed it: its command.failed
    /// goes the way the core's events go, after this turn (a model may be sending from inside its
    /// own Apply), so the screen waiting for it says "couldn't" instead of waiting forever.
    /// </summary>
    private void NotSent(CoreCommand command, string why)
    {
        var failed = command.NotSent($"couldn't send it: {why}");
        ui.TryEnqueue(() => Received([failed]));
    }

    /// <summary>UI thread: a batch of the core's events.</summary>
    private void Received(IReadOnlyList<InkEvent> batch)
    {
        Store.Apply(batch);
        Observer?.Invoke(batch);
    }

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

    /// <summary>
    /// Whether INK_DATA_DIR moves the library. A moved library never looks at the user's Inkwell
    /// 0.2 data either (Import02Model.Looks).
    /// </summary>
    public static bool IsMoved() => !string.IsNullOrEmpty(Environment.GetEnvironmentVariable("INK_DATA_DIR"));

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
