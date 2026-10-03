// The local log: lines on one line with their source, a size cap with older logs kept, crash
// notes that hold the type, stack and version but never the message, and nothing thrown when the
// folder can't be written.
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests;

public sealed class LocalLogTests : IDisposable
{
    private readonly string folder = Path.Combine(Path.GetTempPath(), "inkwell-locallog-" + Guid.NewGuid().ToString("N"));
    private static readonly DateTimeOffset At = new(2026, 10, 3, 12, 34, 56, TimeSpan.FromHours(2));

    public void Dispose()
    {
        if (Directory.Exists(folder))
        {
            Directory.Delete(folder, recursive: true);
        }
    }

    [Fact]
    public void ALineSaysWhenAndFromWhereOnOneLine()
    {
        var log = new LocalLog(folder, now: () => At);
        log.Write("shell", "the core refused a record.delete command:\r\nsecond line");
        log.Write("core", "[warn ink_ffi] command x failed");
        Assert.Equal(
            ["2026-10-03T12:34:56.000+02:00 shell: the core refused a record.delete command: second line",
             "2026-10-03T12:34:56.000+02:00 core: [warn ink_ffi] command x failed"],
            File.ReadAllLines(log.Path));
        Assert.Equal(Path.Combine(folder, "inkwell.log"), log.Path);
        Assert.Equal((byte)'2', File.ReadAllBytes(log.Path)[0]); // no byte-order mark
    }

    /// <summary>
    /// Of stderr, the core's own lines and a panic's location are kept; anything else, a panic's
    /// message included (it can quote what was said), only by its length.
    /// </summary>
    [Theory]
    [InlineData("[warn ink_ffi::runtime] warming the DictationFinal model failed", "core", "[warn ink_ffi::runtime] warming the DictationFinal model failed")]
    [InlineData("[info ink_ffi::control] meeting detection: offering it", "core", "[info ink_ffi::control] meeting detection: offering it")]
    [InlineData("thread 'meetings' panicked at crates/ink-pipeline/src/final_pass.rs:212:31:", "stderr", "thread 'meetings' panicked at crates/ink-pipeline/src/final_pass.rs:212:31:")]
    [InlineData("byte index 7 is not a char boundary; it is inside 'é' of `the words the user said`", "stderr", "a line that is not the core's, not kept (82 characters)")]
    [InlineData("Unhandled exception. System.FormatException: words", "stderr", "a line that is not the core's, not kept (50 characters)")]
    [InlineData("[warning] something else's format", "stderr", "a line that is not the core's, not kept (33 characters)")]
    public void OfStderrOnlyTheCoresLinesAndAPanicsPlaceAreKept(string line, string source, string text) =>
        Assert.Equal((source, text), LocalLog.FromStderr(line));

    [Fact]
    public void ALibrarysLogIsInItsLogsFolder() =>
        Assert.Equal(Path.Combine(folder, "logs"), LocalLog.In(folder).Directory);

    /// <summary>Past the size the log moves to inkwell.1.log, that one to .2, and the oldest goes.</summary>
    [Fact]
    public void TheLogRotatesAndKeepsTheNewestOlderOnes()
    {
        var log = new LocalLog(folder, maxBytes: 100, keep: 2, now: () => At);
        var line = new string('x', 40); // about 80 bytes with the stamp: one line per file
        for (var i = 1; i <= 5; i++)
        {
            log.Write("shell", $"{i} {line}");
        }
        Assert.Equal(["inkwell.1.log", "inkwell.2.log", "inkwell.log"],
            Directory.GetFiles(folder).Select(Path.GetFileName).Order(StringComparer.Ordinal));
        Assert.Contains("shell: 5 ", File.ReadAllText(log.Path), StringComparison.Ordinal);
        Assert.Contains("shell: 4 ", File.ReadAllText(Path.Combine(folder, "inkwell.1.log")), StringComparison.Ordinal);
        Assert.Contains("shell: 3 ", File.ReadAllText(Path.Combine(folder, "inkwell.2.log")), StringComparison.Ordinal);
    }

    /// <summary>An older log held open stops the move, not the log: lines still land, past the cap.</summary>
    [Fact]
    public void AnOlderLogHeldOpenDoesNotStopTheLog()
    {
        var log = new LocalLog(folder, maxBytes: 100, keep: 1, now: () => At);
        var line = new string('x', 40);
        log.Write("shell", "1 " + line);
        log.Write("shell", "2 " + line); // the first moves to inkwell.1.log
        using (new FileStream(Path.Combine(folder, "inkwell.1.log"), FileMode.Open, FileAccess.Read, FileShare.Read))
        {
            log.Write("shell", "3 " + line); // inkwell.1.log can't be replaced now
            log.Write("shell", "4 " + line);
        }
        var current = File.ReadAllText(log.Path);
        Assert.Contains("shell: 3 ", current, StringComparison.Ordinal);
        Assert.Contains("shell: 4 ", current, StringComparison.Ordinal);
        log.Write("shell", "5 " + line); // free again: it moves
        Assert.Contains("shell: 5 ", File.ReadAllText(log.Path), StringComparison.Ordinal);
        Assert.Contains("shell: 4 ", File.ReadAllText(Path.Combine(folder, "inkwell.1.log")), StringComparison.Ordinal);
    }

    /// <summary>A crash note names the exception and the ones inside it by type, with stacks and the version; never a message.</summary>
    [Fact]
    public void ACrashNoteHoldsTypesStacksAndTheVersionButNoMessage()
    {
        Exception thrown;
        try
        {
            try
            {
                throw new FormatException("the words the user said");
            }
            catch (FormatException inner)
            {
                throw new InvalidOperationException("a path C:\\Users\\someone", inner);
            }
        }
        catch (InvalidOperationException e)
        {
            thrown = e;
        }
        var log = new LocalLog(folder, now: () => At);
        var note = log.WriteCrashNote(thrown, "1.0.0");
        Assert.Equal(Path.Combine(folder, "crash-20261003-123456.txt"), note);
        var text = File.ReadAllText(note!);
        Assert.StartsWith("Inkwell crashed at 2026-10-03T12:34:56+02:00", text, StringComparison.Ordinal);
        Assert.Contains("App version: 1.0.0", text, StringComparison.Ordinal);
        Assert.Contains("System.InvalidOperationException (0x80131509)", text, StringComparison.Ordinal);
        Assert.Contains("System.FormatException (0x80131537)", text, StringComparison.Ordinal);
        Assert.Contains(nameof(ACrashNoteHoldsTypesStacksAndTheVersionButNoMessage), text, StringComparison.Ordinal);
        Assert.DoesNotContain("the words the user said", text, StringComparison.Ordinal);
        Assert.DoesNotContain("someone", text, StringComparison.Ordinal);
        // The log says a crash note was written, and a second handler for the same crash writes none.
        Assert.Contains("shell: crashed (System.InvalidOperationException); crash note crash-20261003-123456.txt", File.ReadAllText(log.Path), StringComparison.Ordinal);
        Assert.Null(log.WriteCrashNote(new InvalidOperationException(), "1.0.0"));
        // After the note the process is ending: .NET's own report on stderr quotes the message, and stays out.
        log.Write("stderr", "Unhandled exception. System.InvalidOperationException: the words the user said");
        Assert.DoesNotContain("the words the user said", File.ReadAllText(log.Path), StringComparison.Ordinal);
        Assert.Contains("App version: development build", LocalLog.CrashNote(new InvalidOperationException(), null, At), StringComparison.Ordinal);
    }

    [Fact]
    public void OnlyTheNewestCrashNotesAreKept()
    {
        Directory.CreateDirectory(folder);
        for (var i = 0; i < LocalLog.CrashNotesKept + 3; i++)
        {
            File.WriteAllText(Path.Combine(folder, $"crash-20260901-0000{i:00}.txt"), "old");
        }
        new LocalLog(folder, now: () => At).WriteCrashNote(new InvalidOperationException(), "1.0.0");
        var kept = Directory.GetFiles(folder, "crash-*.txt").Select(Path.GetFileName).Order(StringComparer.Ordinal).ToList();
        Assert.Equal(LocalLog.CrashNotesKept, kept.Count);
        Assert.Equal("crash-20261003-123456.txt", kept[^1]);
        Assert.DoesNotContain("crash-20260901-000000.txt", kept);
    }

    /// <summary>A folder that can't be made (a file is in the way) drops the line; nothing throws.</summary>
    [Fact]
    public void AnUnwritableFolderDropsLinesWithoutThrowing()
    {
        File.WriteAllText(folder, "a file where the folder would be");
        try
        {
            var log = new LocalLog(Path.Combine(folder, "logs"));
            log.Write("shell", "dropped");
            Assert.Null(log.WriteCrashNote(new InvalidOperationException(), null));
        }
        finally
        {
            File.Delete(folder);
        }
    }

    /// <summary>The screens' log reaches the local log once the app has pointed it there.</summary>
    [Fact]
    public void TheScreensLogAlsoGoesWhereTheAppPointsIt()
    {
        var lines = new List<string>();
        var before = ScreenLog.Also;
        ScreenLog.Also = lines.Add;
        try
        {
            ScreenLog.System.Write("a fixed line");
        }
        finally
        {
            ScreenLog.Also = before;
        }
        // Contains: other tests' models write to the same log while this runs.
        Assert.Contains("a fixed line", lines);
    }
}
