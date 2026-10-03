// The app's own diagnostics, on this PC only: a rotating log in the library's folder ("logs"),
// and a short crash note beside it when the app dies of an exception. Nothing here is sent
// anywhere. What goes in is the shell's ScreenLog (command names and fixed words) and the core's
// log lines (which by the core's rule never hold what was said); a crash note holds the
// exception's type, its stack and the app's version, never its message (a message can quote a
// value).
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace Inkwell.Core;

/// <summary>The local log and crash notes. Any thread.</summary>
public sealed partial class LocalLog
{
    /// <summary>The folder in the library's folder that holds the log and the crash notes.</summary>
    public const string FolderName = "logs";

    /// <summary>The current log's name; older ones are inkwell.1.log (newest) to inkwell.N.log.</summary>
    public const string FileName = "inkwell.log";

    /// <summary>How many crash notes are kept, newest first.</summary>
    public const int CrashNotesKept = 10;

    // No byte-order mark: a log is read with anything.
    private static readonly UTF8Encoding Utf8 = new(false);

    private readonly object gate = new();
    private readonly long maxBytes;
    private readonly int keep;
    private readonly Func<DateTimeOffset> now;
    private int crashNoted;
    // Set once a crash note is written: the process is ending, and what it writes to stderr then
    // (.NET's own report quotes the exception's message) stays out of the log.
    private volatile bool ended;

    /// <param name="directory">The "logs" folder (made when the first line is written).</param>
    /// <param name="maxBytes">The size past which the log moves to inkwell.1.log and a new one starts.</param>
    /// <param name="keep">How many older logs are kept.</param>
    /// <param name="now">The clock (tests).</param>
    public LocalLog(string directory, long maxBytes = 1024 * 1024, int keep = 2, Func<DateTimeOffset>? now = null)
    {
        ArgumentException.ThrowIfNullOrEmpty(directory);
        ArgumentOutOfRangeException.ThrowIfLessThan(maxBytes, 1);
        ArgumentOutOfRangeException.ThrowIfNegative(keep);
        Directory = directory;
        this.maxBytes = maxBytes;
        this.keep = keep;
        this.now = now ?? (() => DateTimeOffset.Now);
    }

    /// <summary>The "logs" folder.</summary>
    public string Directory { get; }

    /// <summary>The current log.</summary>
    public string Path => System.IO.Path.Combine(Directory, FileName);

    /// <summary>The "logs" folder of the library in <paramref name="libraryDirectory"/>.</summary>
    public static LocalLog In(string libraryDirectory) => new(System.IO.Path.Combine(libraryDirectory, FolderName));

    /// <summary>
    /// Appends one line: the time, <paramref name="source"/> ("shell", "core") and the message on
    /// one line. A line that can't be written is dropped: there is nowhere left to report it.
    /// </summary>
    public void Write(string source, string message)
    {
        if (ended)
        {
            return;
        }
        Append(source, message);
    }

    private void Append(string source, string message)
    {
        var line = $"{Stamp()} {source}: {OneLine(message)}{Environment.NewLine}";
        lock (gate)
        {
            try
            {
                System.IO.Directory.CreateDirectory(Directory);
                var current = new FileInfo(Path);
                if (current.Exists && current.Length + Utf8.GetByteCount(line) > maxBytes)
                {
                    Rotate();
                }
                File.AppendAllText(Path, line, Utf8);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
            }
        }
    }

    /// <summary>
    /// Writes a crash note for <paramref name="exception"/> (the first one only: a crash can reach
    /// more than one handler) and says so in the log. Returns the note's path, or null when none
    /// was written.
    /// </summary>
    public string? WriteCrashNote(Exception exception, string? appVersion)
    {
        ArgumentNullException.ThrowIfNull(exception);
        if (Interlocked.Exchange(ref crashNoted, 1) == 1)
        {
            return null;
        }
        var at = now();
        var note = System.IO.Path.Combine(Directory, $"crash-{at.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture)}.txt");
        try
        {
            System.IO.Directory.CreateDirectory(Directory);
            File.WriteAllText(note, CrashNote(exception, appVersion, at), Utf8);
            PruneCrashNotes();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            ended = true;
            return null;
        }
        Append("shell", $"crashed ({exception.GetType().FullName}); crash note {System.IO.Path.GetFileName(note)}");
        ended = true;
        return note;
    }

    /// <summary>
    /// What of a line that arrived on stderr goes in the log, and from whom. The core's own lines
    /// ("[warn ink_ffi] ...", through its filtered logger) as they are. A Rust panic's first line
    /// ("thread 'x' panicked at src/a.rs:1:2:") as it is: it names only where. Anything else, a
    /// panic's message included (Rust's default hook prints the payload, which can quote what was
    /// said), only by its length.
    /// </summary>
    public static (string Source, string Text) FromStderr(string line)
    {
        ArgumentNullException.ThrowIfNull(line);
        string[] levels = ["[error ", "[warn ", "[info ", "[debug ", "[trace "];
        if (levels.Any(l => line.StartsWith(l, StringComparison.Ordinal)))
        {
            return ("core", line);
        }
        return PanicAt().IsMatch(line)
            ? ("stderr", line)
            : ("stderr", $"a line that is not the core's, not kept ({line.Length} characters)");
    }

    [GeneratedRegex(@"^thread '[^']*' panicked at [^\s]+:\d+:\d+:$", RegexOptions.CultureInvariant)]
    private static partial Regex PanicAt();

    /// <summary>A crash note's text: when, the app's version, Windows' version, then each exception's type and stack.</summary>
    public static string CrashNote(Exception exception, string? appVersion, DateTimeOffset at)
    {
        ArgumentNullException.ThrowIfNull(exception);
        var text = new StringBuilder();
        text.AppendLine(CultureInfo.InvariantCulture, $"Inkwell crashed at {at.ToString("yyyy-MM-ddTHH:mm:sszzz", CultureInfo.InvariantCulture)}");
        text.AppendLine(CultureInfo.InvariantCulture, $"App version: {appVersion ?? "development build"}");
        text.AppendLine(CultureInfo.InvariantCulture, $"Windows: {Environment.OSVersion.Version}");
        // The exception and every inner one, by type and stack only.
        var seen = new HashSet<Exception>(ReferenceEqualityComparer.Instance);
        var pending = new Queue<Exception>([exception]);
        while (pending.TryDequeue(out var e) && seen.Add(e))
        {
            text.AppendLine();
            text.AppendLine(CultureInfo.InvariantCulture, $"{e.GetType().FullName} (0x{e.HResult:X8})");
            text.AppendLine(e.StackTrace ?? "   (no stack)");
            if (e is AggregateException aggregate)
            {
                foreach (var inner in aggregate.InnerExceptions)
                {
                    pending.Enqueue(inner);
                }
            }
            else if (e.InnerException is { } inner)
            {
                pending.Enqueue(inner);
            }
        }
        return text.ToString();
    }

    private string Stamp() => now().ToString("yyyy-MM-ddTHH:mm:ss.fffzzz", CultureInfo.InvariantCulture);

    private static string OneLine(string message) => message.ReplaceLineEndings(" ");

    /// <summary>inkwell.log becomes inkwell.1.log, each older one moves up one, and the oldest past <c>keep</c> goes.</summary>
    private void Rotate()
    {
        if (keep == 0)
        {
            File.Delete(Path);
            return;
        }
        File.Delete(Older(keep));
        for (var i = keep - 1; i >= 1; i--)
        {
            if (File.Exists(Older(i)))
            {
                File.Move(Older(i), Older(i + 1));
            }
        }
        File.Move(Path, Older(1));
    }

    private string Older(int n) => System.IO.Path.Combine(Directory, $"inkwell.{n}.log");

    private void PruneCrashNotes()
    {
        // The names sort by time.
        foreach (var old in System.IO.Directory.GetFiles(Directory, "crash-*.txt").OrderDescending(StringComparer.Ordinal).Skip(CrashNotesKept))
        {
            File.Delete(old);
        }
    }
}
