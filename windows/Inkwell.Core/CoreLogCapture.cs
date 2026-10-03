// The core writes its log lines to stderr, which an app with no console drops. This points the
// process's stderr at a pipe before the core starts and hands each line to the local log
// (LocalLog), on a thread of its own. The core's Rust standard library asks Windows for the
// stderr handle at every write, so it writes into the pipe from then on. Nothing reads what the
// lines say: they go to the file as they are (the core keeps what was said out of them).
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Inkwell.Core;

public static partial class CoreLogCapture
{
    private const int StdErrorHandle = -12;
    // Enough that the core never waits on a slow disk for a burst of lines.
    private const int PipeBytes = 64 * 1024;

    /// <summary>
    /// Sends every line the process writes to stderr to <paramref name="line"/>, from now on.
    /// False when the pipe could not be made (stderr stays where it was).
    /// </summary>
    public static bool Start(Action<string> line)
    {
        ArgumentNullException.ThrowIfNull(line);
        if (!CreatePipe(out var read, out var write, 0, PipeBytes))
        {
            return false;
        }
        if (!SetStdHandle(StdErrorHandle, write))
        {
            read.Dispose();
            CloseHandle(write);
            return false;
        }
        // The write end stays open for the life of the process: the core writes through it.
        new Thread(() => Pump(read, line)) { IsBackground = true, Name = "Inkwell core log" }.Start();
        return true;
    }

    private static void Pump(SafeFileHandle read, Action<string> line)
    {
        using var stream = new FileStream(read, FileAccess.Read, 1);
        using var reader = new StreamReader(stream, new UTF8Encoding(false));
        try
        {
            while (reader.ReadLine() is { } text)
            {
                line(text);
            }
        }
        catch (IOException)
        {
            // The pipe broke: the process is ending.
        }
    }

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CreatePipe(out SafeFileHandle read, out nint write, nint attributes, int size);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool SetStdHandle(int which, nint handle);

    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool CloseHandle(nint handle);
}
