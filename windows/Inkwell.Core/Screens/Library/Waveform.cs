// The player's two-lane waveform, as the Mac's Waveform: them above, you below, one peak per
// bucket of the record's timeline. Built off the UI thread by reading the chunks a second at a
// time: an hour of audio is never in memory, only its few hundred peaks.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>Peaks per side, 0 to 1, one per bucket.</summary>
/// <param name="Partial">A chunk could not be read: its stretch is flat, which is not silence. The player bar says so.</param>
public sealed record Waveform(IReadOnlyList<float> You, IReadOnlyList<float> Them, bool Partial = false)
{
    public static Waveform Empty { get; } = new([], []);

    /// <summary>Buckets in the player bar's waveform: one bar each.</summary>
    public const int PlayerBars = 160;

    public bool Equals(Waveform? other) =>
        other is not null && Partial == other.Partial && You.SequenceEqual(other.You) && Them.SequenceEqual(other.Them);

    public override int GetHashCode() => HashCode.Combine(You.Count, Them.Count, Partial);

    /// <summary>
    /// <b>Background.</b> Reads <paramref name="chunks"/> and peaks them into
    /// <paramref name="buckets"/> over <paramref name="durationMs"/>. Each lane is scaled to its
    /// own loudest bucket (the far end is often quieter), with a square root so quiet speech still
    /// shows. A chunk that cannot be read leaves its buckets flat, is logged (its side and the kind
    /// of failure, never a path or samples) and marks the waveform partial. Returns
    /// <see cref="Empty"/> once <paramref name="cancelled"/> says so.
    /// </summary>
    public static Waveform Build(
        IReadOnlyList<TimelineChunk> chunks, long durationMs, int buckets, Func<bool>? cancelled = null, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(chunks);
        if (buckets <= 0 || durationMs <= 0)
        {
            return Empty;
        }
        var lanes = new Dictionary<Channel, float[]> { [Channel.Mic] = new float[buckets], [Channel.Far] = new float[buckets] };
        var partial = false;
        var msPerBucket = (double)durationMs / buckets;
        foreach (var chunk in chunks.Where(c => c.Frames > 0 && c.SampleRate > 0))
        {
            var cursor = new SliceCursor([chunk], chunk.StartMs);
            while (cursor.Next(1) is { } slice)
            {
                if (cancelled?.Invoke() == true)
                {
                    return Empty;
                }
                float[] samples;
                try
                {
                    samples = ChunkAudio.Samples(slice);
                }
                catch (ChunkReadException e)
                {
                    (log ?? ScreenLog.System).Write(
                        $"waveform: a {ChunkAudio.SideName(chunk.Channel)} chunk could not be read ({e.Error}); drawn flat");
                    partial = true;
                    break;
                }
                var channels = Math.Max(chunk.Channels, 1);
                var frames = samples.Length / channels;
                var rate = (double)chunk.SampleRate;
                var lane = lanes[chunk.Channel];
                for (var f = 0; f < frames; f++)
                {
                    var peak = 0f;
                    for (var c = 0; c < channels; c++)
                    {
                        peak = Math.Max(peak, Math.Abs(samples[f * channels + c]));
                    }
                    var ms = slice.StartSeconds * 1000 + f * 1000 / rate;
                    var bucket = (int)Math.Floor(ms / msPerBucket);
                    if (bucket >= 0 && bucket < buckets && peak > lane[bucket])
                    {
                        lane[bucket] = peak;
                    }
                }
            }
        }
        static float[] Scaled(float[] lane)
        {
            var top = lane.Max();
            return top > 0 ? lane.Select(v => MathF.Sqrt(v / top)).ToArray() : lane;
        }
        return new Waveform(Scaled(lanes[Channel.Mic]), Scaled(lanes[Channel.Far]), partial);
    }
}

/// <summary>
/// Builds the open record's waveform off the UI thread, one record at a time: a new record
/// cancels the build under way, and a result that arrives for a record no longer shown is dropped,
/// so a quick switch never shows the previous record's waveform. Delivered on the thread that
/// called <see cref="Load"/> when it has a SynchronizationContext (the UI thread).
/// </summary>
public sealed class WaveformLoader(ScreenLog? log = null) : ObservableModel
{
    private readonly ScreenLog _log = log ?? ScreenLog.System;
    private Ticket? _ticket;
    private Task _delivery = Task.CompletedTask;

    public Waveform Waveform { get; private set; } = Waveform.Empty;

    /// <summary>The line the player shows when the waveform could not be built.</summary>
    public const string FailedText = "Couldn't draw this recording's waveform.";

    /// <summary>Why the waveform is missing, when its build failed; null otherwise.</summary>
    public string? Failure { get; private set; }

    /// <summary>The record <see cref="Waveform"/> is (or is being built) for.</summary>
    public string? Record { get; private set; }

    /// <summary>Starts building <paramref name="document"/>'s waveform, replacing whatever was being built.</summary>
    public void Load(RecordDocument document, int buckets = Waveform.PlayerBars)
    {
        ArgumentNullException.ThrowIfNull(document);
        Cancel();
        var id = document.Record.Record;
        Record = id;
        Waveform = Waveform.Empty;
        Failure = null;
        Changed();
        var ticket = new Ticket();
        _ticket = ticket;
        var chunks = document.Chunks;
        var duration = document.DurationMs;
        var log = _log;
        // A second of audio at a time, on the thread pool; the ticket lets a newer record stop it.
        var build = Task.Run(() => Waveform.Build(chunks, duration, buckets, () => ticket.Cancelled, log));
        _delivery = Deliver(build, id, ticket);
    }

    /// <summary>Stops the build under way (the record closed, or another opened).</summary>
    public void Cancel()
    {
        if (_ticket is not null)
        {
            _ticket.Cancelled = true;
        }
        _ticket = null;
    }

    /// <summary>Completes once the latest build has been delivered or dropped (tests).</summary>
    public Task Settled() => _delivery;

    private async Task Deliver(Task<Waveform> build, string id, Ticket ticket)
    {
        Waveform built;
        try
        {
            built = await build.ConfigureAwait(true);
        }
        catch (Exception e)
        {
            // Said on screen and logged by kind, never left as an empty lane that looks like silence.
            _log.Write($"waveform: the build failed ({e.GetType().Name})");
            if (!ticket.Cancelled && Record == id)
            {
                Failure = FailedText;
                Changed();
            }
            return;
        }
        if (ticket.Cancelled || Record != id)
        {
            return;
        }
        Waveform = built;
        Changed();
    }

    private sealed class Ticket
    {
        private volatile bool _cancelled;

        public bool Cancelled
        {
            get => _cancelled;
            set => _cancelled = value;
        }
    }
}
