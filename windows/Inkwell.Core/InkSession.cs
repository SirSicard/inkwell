// C# over the core's C ABI (inkwell.h, through the csbindgen declarations in Generated/). The
// header holds the contract; in short:
//
// - Events arrive on ONE core thread ("ink-events"). InkSession.Start's handler runs there: hop to
//   the UI thread (EventRelay) and return promptly. Never call Shutdown from it.
// - The core installs the only logger for its code: do not install one for the core's targets.
//   Event payloads can carry the user's words; never log them.
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Native;

// Every native call passes blittable types only: no runtime marshalling, nothing for NativeAOT to
// generate at run time.
[assembly: DisableRuntimeMarshalling]

namespace Inkwell.Core;

/// <summary>A status the core returned (INK_ERR_* in inkwell.h).</summary>
public sealed class InkStatusException(int code) : Exception(Describe(code))
{
    /// <summary>The code.</summary>
    public int Code { get; } = code;

    private static string Describe(int code) => code switch
    {
        InkStatus.NotInitialized => "the core is not running",
        InkStatus.AlreadyInitialized => "the core is already running",
        InkStatus.InvalidArgument => "the core could not read the argument",
        InkStatus.Failed => "the core could not do it",
        InkStatus.UnknownCall => "no such call is waiting",
        InkStatus.Panic => "a bug in the core (contained)",
        _ => $"core status {code}",
    };

    internal static void Check(int status)
    {
        if (status != InkStatus.Ok)
        {
            throw new InkStatusException(status);
        }
    }
}

/// <summary>ink_init's configuration.</summary>
/// <param name="DataDir">The library, recordings and models live here (absolute).</param>
/// <param name="ModelsDir">Models elsewhere (absolute), or null for DataDir\models.</param>
/// <param name="LogLevel">off, error, warn, info, debug or trace; null for info.</param>
/// <param name="LogStderr">Whether the core writes log lines to stderr; null for yes.</param>
public sealed record InkConfig(
    string DataDir, string? ModelsDir = null, string? LogLevel = null, bool? LogStderr = null)
{
    internal byte[] ToJson()
    {
        var buffer = new System.Buffers.ArrayBufferWriter<byte>();
        using (var w = new Utf8JsonWriter(buffer))
        {
            w.WriteStartObject();
            w.WriteString("data_dir", DataDir);
            if (ModelsDir is not null)
            {
                w.WriteString("models_dir", ModelsDir);
            }
            if (LogLevel is not null)
            {
                w.WriteString("log_level", LogLevel);
            }
            if (LogStderr is bool stderr)
            {
                w.WriteBoolean("log_stderr", stderr);
            }
            w.WriteEndObject();
        }
        return buffer.WrittenSpan.ToArray();
    }
}

/// <summary>One stream's audio bands (inkwell.h, InkBands): RMS amplitude per band, linear full scale.</summary>
/// <param name="Low">80-500 Hz.</param>
/// <param name="Mid">500 Hz-2 kHz.</param>
/// <param name="High">2-8 kHz.</param>
/// <param name="Published">Bands published so far.</param>
public readonly record struct AudioBands(float Low, float Mid, float High, ulong Published);

/// <summary>
/// The running core. One per process; call <see cref="Shutdown"/> on every quit path (a loaded
/// model must be dropped before the process exits).
/// </summary>
public sealed unsafe class InkSession : IDisposable
{
    /// <summary>Holds the event handler for the event thread.</summary>
    private sealed class EventSink(Action<InkEvent> handler)
    {
        public readonly Action<InkEvent> Handler = handler;
        /// <summary>The managed id of the core's event thread, once it has called in.</summary>
        public volatile int EventThread;
    }

    /// <summary>INK_ABI_VERSION: core.ready must report this.</summary>
    public const long AbiVersion = InkAbi.Version;

    private readonly EventSink sink;
    private readonly GCHandle handle;
    private readonly Lock stopLock = new();
    private bool stopped;

    private InkSession(EventSink sink, GCHandle handle)
    {
        this.sink = sink;
        this.handle = handle;
    }

    /// <summary>
    /// Starts the core. <paramref name="onEvent"/> runs on the core's event thread, in order (see
    /// the file header). It must not throw: an exception escaping it ends the process.
    /// </summary>
    public static InkSession Start(InkConfig config, Action<InkEvent> onEvent)
    {
        ArgumentNullException.ThrowIfNull(config);
        ArgumentNullException.ThrowIfNull(onEvent);
        var sink = new EventSink(onEvent);
        // Freed only after ink_shutdown returns, when no callback can arrive any more.
        var handle = GCHandle.Alloc(sink);
        int status;
        fixed (byte* json = Terminated(config.ToJson()))
        {
            status = NativeMethods.ink_init(json, &OnEvent, (void*)GCHandle.ToIntPtr(handle));
        }
        if (status != InkStatus.Ok)
        {
            handle.Free();
            throw new InkStatusException(status);
        }
        return new InkSession(sink, handle);
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static void OnEvent(void* ctx, byte* json, nuint len)
    {
        if (ctx is null || json is null)
        {
            return;
        }
        var sink = (EventSink)GCHandle.FromIntPtr((nint)ctx).Target!;
        sink.EventThread = Environment.CurrentManagedThreadId;
        // Copied: the core's string is valid only during this call.
        var bytes = new ReadOnlySpan<byte>(json, checked((int)len)).ToArray();
        InkEvent evt;
        try
        {
            evt = InkEvent.Decode(bytes);
        }
        catch (Exception)
        {
            // Only JSON without a string "type" fails to decode at all (a known event whose
            // content does not decode arrives as UndecodableEvent with its type and record).
            // Anything else thrown here must not cross into the core either: the shell shows an
            // undecodable event instead.
            evt = new UndecodableEvent { Type = "" };
        }
        try
        {
            sink.Handler(evt);
        }
        catch (Exception e)
        {
            // An exception cannot cross into the core. Named by type only: its message could quote
            // an event's words.
            Environment.FailFast($"the core's event handler threw {e.GetType().FullName}");
        }
    }

    /// <summary>Queues a command (see inkwell.h for the commands). Its outcome arrives as events.</summary>
    public void Command(string json)
    {
        ArgumentNullException.ThrowIfNull(json);
        fixed (byte* p = Terminated(Encoding.UTF8.GetBytes(json)))
        {
            InkStatusException.Check(NativeMethods.ink_command(p));
        }
    }

    /// <summary>Queues a command built from <paramref name="fields"/> (strings only; "cmd" among them).</summary>
    public void Command(IReadOnlyDictionary<string, string> fields)
    {
        ArgumentNullException.ThrowIfNull(fields);
        var buffer = new System.Buffers.ArrayBufferWriter<byte>();
        using (var w = new Utf8JsonWriter(buffer))
        {
            w.WriteStartObject();
            foreach (var (key, value) in fields)
            {
                w.WriteString(key, value);
            }
            w.WriteEndObject();
        }
        fixed (byte* p = Terminated(buffer.WrittenSpan))
        {
            InkStatusException.Check(NativeMethods.ink_command(p));
        }
    }

    /// <summary>
    /// Stops the core: every worker, every engine, every model. When it returns, from whichever
    /// caller, the core has stopped and no event arrives any more. Safe to call twice and from
    /// several threads: later callers wait for the first. Never from the event handler (the core
    /// waits for that thread to finish, and it would wait here): that throws.
    /// </summary>
    public void Shutdown()
    {
        if (sink.EventThread == Environment.CurrentManagedThreadId)
        {
            throw new InvalidOperationException("Shutdown called on the core's event thread");
        }
        // Held across ink_shutdown, so a caller that arrives meanwhile waits for it to finish.
        lock (stopLock)
        {
            if (stopped)
            {
                return;
            }
            // INK_OK, or INK_ERR_NOT_INITIALIZED when the core had already stopped: either way it
            // is stopped now.
            _ = NativeMethods.ink_shutdown();
            stopped = true;
            handle.Free();
        }
    }

    /// <inheritdoc cref="Shutdown"/>
    public void Dispose() => Shutdown();

    /// <summary>
    /// The latest bands of the live audio (your mic while a take is open), copied out. Any thread,
    /// any rate: ink_bands_read never locks or allocates, so the ink reads it once per frame.
    /// Zeros before the core starts and while nothing is live.
    /// </summary>
    public static AudioBands Bands()
    {
        var bands = default(InkBands);
        _ = NativeMethods.ink_bands_read(&bands);
        return new AudioBands(bands.low, bands.mid, bands.high, bands.published);
    }

    /// <summary>The far end's latest bands during a meeting, as <see cref="Bands"/> gives the mic's.</summary>
    public static AudioBands FarBands()
    {
        var bands = default(InkBands);
        _ = NativeMethods.ink_far_bands_read(&bands);
        return new AudioBands(bands.low, bands.mid, bands.high, bands.published);
    }

    /// <summary>A NUL-terminated copy: every string crossing the ABI is.</summary>
    private static byte[] Terminated(ReadOnlySpan<byte> utf8)
    {
        var bytes = new byte[utf8.Length + 1];
        utf8.CopyTo(bytes);
        return bytes;
    }
}
