// A record's audio as the player reads it, as the Mac's ChunkAudio: the core's chunk files (raw
// little-endian float samples, interleaved, after a header the core names the size of), read a
// few seconds at a time. Disk is where the audio lives; the player never holds more than the
// slices it has queued. Plain file IO, so it runs anywhere .NET does.
using System.Buffers.Binary;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>A stretch of one chunk.</summary>
public sealed record ChunkSlice(TimelineChunk Chunk, long FirstFrame, long FrameCount)
{
    /// <summary>Where it starts on the record's timeline, in seconds.</summary>
    public double StartSeconds => Chunk.StartMs / 1000.0 + (double)FirstFrame / Math.Max(Chunk.SampleRate, 1);

    /// <summary>Where it ends on the record's timeline, ms.</summary>
    public long EndMs => (long)((StartSeconds + (double)FrameCount / Math.Max(Chunk.SampleRate, 1)) * 1000);
}

/// <summary>
/// Walks one side's chunks from a moment on, a slice at a time. Gaps between chunks (a side that
/// started late, or a stretch lost) are skipped: each slice carries its own place on the timeline,
/// and silence fills the rest.
/// </summary>
public sealed class SliceCursor
{
    private readonly List<TimelineChunk> _chunks;
    private int _index;
    private long _frame;

    /// <summary><paramref name="chunks"/> of one side, from <paramref name="ms"/> on the record's timeline.</summary>
    public SliceCursor(IEnumerable<TimelineChunk> chunks, long ms)
    {
        ArgumentNullException.ThrowIfNull(chunks);
        _chunks = chunks.Where(c => c.Frames > 0 && c.SampleRate > 0).OrderBy(c => c.StartMs).ToList();
        _index = _chunks.FindIndex(c => c.EndMs > ms);
        if (_index < 0)
        {
            _index = _chunks.Count;
        }
        else
        {
            var chunk = _chunks[_index];
            _frame = Math.Max(0, (ms - chunk.StartMs) * chunk.SampleRate / 1000);
        }
    }

    /// <summary>The next slice, at most <paramref name="seconds"/> long; null at the end.</summary>
    public ChunkSlice? Next(double seconds)
    {
        while (_index < _chunks.Count)
        {
            var chunk = _chunks[_index];
            if (_frame >= chunk.Frames)
            {
                _index++;
                _frame = 0;
                continue;
            }
            var count = Math.Min((long)(seconds * chunk.SampleRate), chunk.Frames - _frame);
            var slice = new ChunkSlice(chunk, _frame, Math.Max(count, 1));
            _frame += slice.FrameCount;
            return slice;
        }
        return null;
    }
}

/// <summary>Samples, one array per channel (deinterleaved), at a rate.</summary>
public sealed class PcmBuffer
{
    public PcmBuffer(int sampleRate, float[][] channels)
    {
        ArgumentNullException.ThrowIfNull(channels);
        SampleRate = sampleRate;
        Channels = channels;
    }

    public int SampleRate { get; }

    /// <summary>One array per channel, each <see cref="Frames"/> long.</summary>
    public IReadOnlyList<float[]> Channels { get; }

    public int Frames => Channels.Count > 0 ? Channels[0].Length : 0;

    public double Seconds => SampleRate > 0 ? (double)Frames / SampleRate : 0;
}

/// <summary>Why a chunk could not be read. Names the kind of failure, never a path or a sample.</summary>
public enum ChunkReadError
{
    /// <summary>The file could not be opened or read.</summary>
    Unreadable,
    /// <summary>The file holds fewer frames than the chunk says.</summary>
    Truncated,
}

public sealed class ChunkReadException : IOException
{
    public ChunkReadException()
        : this(ChunkReadError.Unreadable)
    {
    }

    public ChunkReadException(string message)
        : base(message)
    {
    }

    public ChunkReadException(string message, Exception innerException)
        : base(message, innerException)
    {
    }

    public ChunkReadException(ChunkReadError error, Exception? inner = null)
        : base(error == ChunkReadError.Truncated ? "a chunk file is shorter than its frames" : "a chunk file could not be read", inner)
    {
        Error = error;
    }

    public ChunkReadError Error { get; }
}

public static class ChunkAudio
{
    /// <summary>The interleaved samples of <paramref name="slice"/>, as stored.</summary>
    /// <exception cref="ChunkReadException">The file could not be read, or is shorter than the slice.</exception>
    public static float[] Samples(ChunkSlice slice)
    {
        ArgumentNullException.ThrowIfNull(slice);
        var chunk = slice.Chunk;
        var bytesPerFrame = (long)Math.Max(chunk.Channels, 1) * 4;
        var wanted = slice.FrameCount * bytesPerFrame;
        if (chunk.DataOffset < 0 || slice.FirstFrame < 0 || wanted <= 0 || wanted > int.MaxValue)
        {
            throw new ChunkReadException(ChunkReadError.Unreadable);
        }
        var data = new byte[wanted];
        int read;
        try
        {
            using var file = new FileStream(chunk.Path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            file.Seek(chunk.DataOffset + slice.FirstFrame * bytesPerFrame, SeekOrigin.Begin);
            read = file.ReadAtLeast(data, data.Length, throwOnEndOfStream: false);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            throw new ChunkReadException(ChunkReadError.Unreadable, e);
        }
        // Fewer bytes than the slice's frames (a truncated file) would put everything after it
        // early by the difference, since the cursor moves on by the full count: an error instead.
        if (read != data.Length)
        {
            throw new ChunkReadException(ChunkReadError.Truncated);
        }
        var samples = new float[data.Length / 4];
        for (var i = 0; i < samples.Length; i++)
        {
            samples[i] = BinaryPrimitives.ReadSingleLittleEndian(data.AsSpan(i * 4, 4));
        }
        return samples;
    }

    /// <summary><paramref name="slice"/> in its chunk's format, deinterleaved.</summary>
    public static PcmBuffer Buffer(ChunkSlice slice)
    {
        ArgumentNullException.ThrowIfNull(slice);
        var samples = Samples(slice);
        var channels = Math.Max(slice.Chunk.Channels, 1);
        var frames = samples.Length / channels;
        var lanes = new float[channels][];
        for (var c = 0; c < channels; c++)
        {
            var lane = new float[frames];
            for (var f = 0; f < frames; f++)
            {
                lane[f] = samples[f * channels + c];
            }
            lanes[c] = lane;
        }
        return new PcmBuffer(slice.Chunk.SampleRate, lanes);
    }

    /// <summary>
    /// <paramref name="buffer"/> at <paramref name="sampleRate"/> with <paramref name="channels"/>
    /// channels (a side whose device changed rate mid-meeting): linear interpolation between
    /// samples; mono is spread to every channel, many channels to mono are averaged.
    /// </summary>
    public static PcmBuffer Convert(PcmBuffer buffer, int sampleRate, int channels)
    {
        ArgumentNullException.ThrowIfNull(buffer);
        ArgumentOutOfRangeException.ThrowIfLessThan(sampleRate, 1);
        ArgumentOutOfRangeException.ThrowIfLessThan(channels, 1);
        if (buffer.SampleRate == sampleRate && buffer.Channels.Count == channels)
        {
            return buffer;
        }
        var source = buffer.Channels.Count == 0 ? [Array.Empty<float>()] : buffer.Channels;
        float[] Lane(int c)
        {
            if (source.Count == channels)
            {
                return source[c];
            }
            if (source.Count == 1)
            {
                return source[0];
            }
            if (channels == 1)
            {
                var mixed = new float[buffer.Frames];
                foreach (var lane in source)
                {
                    for (var f = 0; f < mixed.Length; f++)
                    {
                        mixed[f] += lane[f] / source.Count;
                    }
                }
                return mixed;
            }
            return c < source.Count ? source[c] : new float[buffer.Frames];
        }
        var frames = buffer.SampleRate == sampleRate
            ? buffer.Frames
            : (int)Math.Round((double)buffer.Frames * sampleRate / Math.Max(buffer.SampleRate, 1));
        var output = new float[channels][];
        for (var c = 0; c < channels; c++)
        {
            output[c] = Resample(Lane(c), frames);
        }
        return new PcmBuffer(sampleRate, output);
    }

    private static float[] Resample(float[] lane, int frames)
    {
        if (lane.Length == frames)
        {
            return lane;
        }
        var output = new float[frames];
        if (lane.Length == 0)
        {
            return output;
        }
        var step = frames > 1 ? (double)(lane.Length - 1) / (frames - 1) : 0;
        for (var f = 0; f < frames; f++)
        {
            var at = f * step;
            var i = (int)at;
            var next = Math.Min(i + 1, lane.Length - 1);
            output[f] = (float)(lane[i] + (lane[next] - lane[i]) * (at - i));
        }
        return output;
    }

    /// <summary>A side's name for a log line.</summary>
    internal static string SideName(Channel side) => side == Channel.Mic ? "mic" : "far";
}
