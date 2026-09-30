// The C ABI from C#, end to end: init -> core.ready -> a replay command -> the record event ->
// shutdown. As the Swift smoke test, minus the external engine (engines registered from C# come
// with the Windows engines).
//
// Needs the core's DLL: `cargo build -p ink-ffi --lib` in core/ (Directory.Build.props copies
// core/target/debug/ink_ffi.dll next to the tests).
using System.Collections.Concurrent;
using Inkwell.Core.Events;
using Inkwell.Core.Native;
using Xunit;

namespace Inkwell.Core.Tests;

/// <summary>Every event, in order, with the managed id of the thread that delivered it.</summary>
internal sealed class Events
{
    private readonly ConcurrentQueue<(InkEvent Event, int Thread)> all = new();

    public void Record(InkEvent e) => all.Enqueue((e, Environment.CurrentManagedThreadId));

    public IReadOnlyList<InkEvent> All => all.Select(x => x.Event).ToList();

    public IReadOnlyList<int> Threads => all.Select(x => x.Thread).ToList();

    /// <summary>Waits for the first event of type T; looks at least once.</summary>
    public T? Wait<T>(TimeSpan timeout) where T : InkEvent
    {
        var until = DateTime.UtcNow + timeout;
        while (true)
        {
            if (all.Select(x => x.Event).OfType<T>().FirstOrDefault() is T found)
            {
                return found;
            }
            if (DateTime.UtcNow >= until)
            {
                return null;
            }
            Thread.Sleep(10);
        }
    }
}

[Collection(RealCore.Name)]
public class SmokeTests
{
    private static string RepoRoot()
    {
        // From the test's output directory up to the directory holding windows/.
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !Directory.Exists(Path.Combine(dir.FullName, "fixtures", "ami")))
        {
            dir = dir.Parent;
        }
        return dir?.FullName ?? throw new DirectoryNotFoundException("no fixtures/ami above the test");
    }

    [Fact]
    public void InitReplayRecordShutdown()
    {
        Assert.True(
            File.Exists(Path.Combine(AppContext.BaseDirectory, "ink_ffi.dll")),
            "ink_ffi.dll is missing: run `cargo build -p ink-ffi --lib` in core/, then rebuild the tests");
        var fixtures = Path.Combine(RepoRoot(), "fixtures", "ami");
        var mic = Path.Combine(fixtures, "IS1009a-mic.wav");
        var far = Path.Combine(fixtures, "IS1009a-far.wav");
        Assert.True(File.Exists(mic), mic);
        var data = Path.Combine(Path.GetTempPath(), $"inkwell-smoke-{Guid.NewGuid():N}");
        try
        {
            // init
            var events = new Events();
            var session = InkSession.Start(new InkConfig(data, LogLevel: "warn", LogStderr: true), events.Record);
            var ready = events.Wait<CoreReady>(TimeSpan.FromSeconds(5));
            Assert.NotNull(ready);
            Assert.Equal(InkAbi.Version, (uint)ready.Abi);
            Assert.Throws<InkStatusException>(() => InkSession.Start(new InkConfig(data), _ => { }));

            // Commands the core cannot read are refused at once, with its status.
            var refused = Assert.Throws<InkStatusException>(() => session.Command("""{"cmd":"fly"}"""));
            Assert.Equal(InkStatus.InvalidArgument, refused.Code);

            // a replay command -> the record event
            session.Command(new Dictionary<string, string>
            {
                ["cmd"] = "replay_meeting", ["mic"] = mic, ["far"] = far, ["title"] = "Smoke", ["pacing"] = "fast",
            });
            var started = events.Wait<MeetingStarted>(TimeSpan.FromSeconds(30));
            Assert.NotNull(started);
            var finished = events.Wait<MeetingFinished>(TimeSpan.FromSeconds(120));
            Assert.True(finished is not null, $"no record: {string.Join(", ", events.All.Select(e => e.Type))}");
            Assert.Equal(started.Record, finished.Record);
            // No engine is installed, so the final pass has nothing to run on: the live record
            // stands, and says so.
            Assert.Equal(finished.Record, events.Wait<MeetingKeptLive>(TimeSpan.Zero)?.Record);
            Assert.DoesNotContain(events.All, e => e is UnknownEvent or UndecodableEvent);
            // Types only: payloads can carry words.
            TestContext.Current.TestOutputHelper?.WriteLine(
                $"smoke: revision {finished.Revision}; events: {string.Join(", ", events.All.Select(e => e.Type).Distinct())}");

            // Events arrive on one core thread, never the test's.
            Assert.Single(events.Threads.Distinct());
            Assert.NotEqual(Environment.CurrentManagedThreadId, events.Threads[0]);

            // shutdown: core.stopped is the last event, and nothing arrives after it returns.
            session.Shutdown();
            Assert.IsType<CoreStopped>(events.All[^1]);
            var count = events.All.Count;
            Thread.Sleep(50);
            Assert.Equal(count, events.All.Count);
            session.Shutdown(); // twice is safe
            var afterStop = Assert.Throws<InkStatusException>(() => session.Command("""{"cmd":"models.list"}"""));
            Assert.Equal(InkStatus.NotInitialized, afterStop.Code);
        }
        finally
        {
            try
            {
                Directory.Delete(data, recursive: true);
            }
            catch (DirectoryNotFoundException)
            {
            }
        }
    }
}
