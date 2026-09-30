// A record's sound on Windows (RecordPlayer's IRecordAudioOutput): a MediaPlayer fed by a
// MediaStreamSource of 16-bit stereo PCM at 48 kHz. The source is a live stream that never seeks:
// each time the lanes start, their clock's zero is the next sample the stream hands out, and the
// SampleRequested handler (on a media thread) mixes both lanes' queued buffers at their places and
// volumes into each 20 ms sample. PlayedSeconds is the MediaPlayer's position past that zero, so
// the playhead follows what is heard, not what was handed out.
//
// Why this route: it is all Windows' own WinRT media API through the SDK projection (no package,
// no raw COM, no reflection), so it publishes with NativeAOT; MediaPlayer follows the default
// output device and does the resampling to it. The system media controls are turned off: this is
// a record's audio, not the user's media.
//
// While paused or closed the MediaPlayer asks for nothing: an idle record costs no CPU.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Microsoft.UI.Dispatching;
using Windows.Media.Core;
using Windows.Media.Devices;
using Windows.Media.MediaProperties;
using Windows.Media.Playback;
using Windows.Security.Cryptography;

namespace Inkwell.Screens;

public sealed class WindowsAudioOutput : IRecordAudioOutput, IDisposable
{
    private const int Rate = 48_000;
    private const int Channels = 2;
    /// <summary>Frames per sample handed to the stream: 20 ms.</summary>
    private const int FramesPerSample = Rate / 50;

    /// <summary>When an output last failed to play: a new one does not try again within this long (a failing device would loop).</summary>
    private static readonly TimeSpan RetryAfterFailure = TimeSpan.FromSeconds(2);
    private static long _lastFailureTicks;

    private readonly DispatcherQueue _ui;
    private readonly ScreenLog _log;
    private readonly object _gate = new();
    /// <summary>Guarded by <see cref="_gate"/>: what the media thread mixes.</summary>
    private readonly List<Block> _blocks = [];
    private readonly Dictionary<Channel, float> _volumes = new() { [Channel.Mic] = 1, [Channel.Far] = 1 };
    private long _served;
    private long _origin;
    private bool _lanesStarted;

    private MediaPlayer? _player;
    private MediaStreamSource? _source;
    private MediaSource? _media;
    private bool _playing;
    private bool _closed;

    public event EventHandler? OutputChanged;

    /// <param name="log">Where a failure is named (never a path or what was said).</param>
    public WindowsAudioOutput(ScreenLog? log = null)
    {
        _ui = DispatcherQueue.GetForCurrentThread()
            ?? throw new InvalidOperationException("the audio output is made on the UI thread");
        _log = log ?? ScreenLog.System;
    }

    /// <summary>A player for <paramref name="document"/> on this PC's output: what LibraryModel's makePlayer returns.</summary>
    public static RecordPlayer PlayerFor(RecordDocument document, ScreenLog? log = null) =>
        new(document, () => new WindowsAudioOutput(log), log: log);

    private sealed record Block(Channel Side, float[][] Lanes, long StartFrame, Action Rendered)
    {
        public long EndFrame => StartFrame + (Lanes.Length > 0 ? Lanes[0].Length : 0);
    }

    public void Open(IReadOnlyList<AudioLane> lanes)
    {
        ObjectDisposedException.ThrowIf(_closed, this);
        if (DateTime.UtcNow.Ticks - Interlocked.Read(ref _lastFailureTicks) < RetryAfterFailure.Ticks)
        {
            throw new InvalidOperationException("the output failed a moment ago");
        }
        if (string.IsNullOrEmpty(MediaDevice.GetDefaultAudioRenderId(AudioDeviceRole.Default)))
        {
            throw new InvalidOperationException("no output device");
        }
        var properties = AudioEncodingProperties.CreatePcm(Rate, Channels, 16);
        var source = new MediaStreamSource(new AudioStreamDescriptor(properties))
        {
            CanSeek = false,
            IsLive = true,
            BufferTime = TimeSpan.Zero,
        };
        source.Starting += (_, args) => args.Request.SetActualStartPosition(TimeSpan.Zero);
        source.SampleRequested += OnSampleRequested;
        var media = MediaSource.CreateFromMediaStreamSource(source);
        var player = new MediaPlayer
        {
            AudioCategory = MediaPlayerAudioCategory.Media,
            RealTimePlayback = true,
            Source = media,
        };
        player.CommandManager.IsEnabled = false;
        player.MediaFailed += OnMediaFailed;
        MediaDevice.DefaultAudioRenderDeviceChanged += OnDefaultDeviceChanged;
        _source = source;
        _media = media;
        _player = player;
    }

    public void Schedule(Channel side, PcmBuffer buffer, double atSeconds, Action rendered)
    {
        ArgumentNullException.ThrowIfNull(buffer);
        ArgumentNullException.ThrowIfNull(rendered);
        var lanes = ChunkAudio.Convert(buffer, Rate, Channels).Channels.ToArray();
        lock (_gate)
        {
            _blocks.Add(new Block(side, lanes, _origin + (long)Math.Round(atSeconds * Rate), rendered));
        }
    }

    public void Start()
    {
        if (_player is not { } player)
        {
            throw new InvalidOperationException("the output is not open");
        }
        lock (_gate)
        {
            // The lanes' zero: the next sample handed out. Blocks queued before this moment were
            // placed from the previous zero, so they move with it.
            var shift = _served - _origin;
            _origin = _served;
            for (var i = 0; i < _blocks.Count; i++)
            {
                _blocks[i] = _blocks[i] with { StartFrame = _blocks[i].StartFrame + shift };
            }
            _lanesStarted = true;
        }
        if (!_playing)
        {
            player.Play();
            _playing = true;
        }
    }

    public void StopLanes()
    {
        lock (_gate)
        {
            _blocks.Clear();
            _lanesStarted = false;
        }
    }

    public void Pause()
    {
        _player?.Pause();
        _playing = false;
    }

    public void SetVolume(Channel side, float volume)
    {
        lock (_gate)
        {
            _volumes[side] = Math.Clamp(volume, 0, 1);
        }
    }

    public double? PlayedSeconds
    {
        get
        {
            if (_player is not { } player || !_playing)
            {
                return null;
            }
            long origin;
            lock (_gate)
            {
                if (!_lanesStarted)
                {
                    return null;
                }
                origin = _origin;
            }
            var position = player.PlaybackSession.Position.TotalSeconds;
            return Math.Max(position - (double)origin / Rate, 0);
        }
    }

    public void Close()
    {
        if (_closed)
        {
            return;
        }
        _closed = true;
        StopLanes();
        MediaDevice.DefaultAudioRenderDeviceChanged -= OnDefaultDeviceChanged;
        if (_player is { } player)
        {
            player.MediaFailed -= OnMediaFailed;
            player.Pause();
            player.Source = null;
            player.Dispose();
        }
        if (_source is { } source)
        {
            source.SampleRequested -= OnSampleRequested;
        }
        _media?.Dispose();
        _player = null;
        _source = null;
        _media = null;
        _playing = false;
    }

    public void Dispose()
    {
        Close();
        GC.SuppressFinalize(this);
    }

    /// <summary>The media thread asks for the next 20 ms: both lanes mixed at their places and volumes.</summary>
    private void OnSampleRequested(MediaStreamSource sender, MediaStreamSourceSampleRequestedEventArgs args)
    {
        var mix = new float[FramesPerSample * Channels];
        var due = new List<Action>();
        long start;
        lock (_gate)
        {
            start = _served;
            var end = start + FramesPerSample;
            if (_lanesStarted)
            {
                foreach (var block in _blocks)
                {
                    if (block.EndFrame <= start || block.StartFrame >= end)
                    {
                        continue;
                    }
                    var volume = _volumes.GetValueOrDefault(block.Side, 1);
                    var from = Math.Max(block.StartFrame, start);
                    var to = Math.Min(block.EndFrame, end);
                    for (var frame = from; frame < to; frame++)
                    {
                        var at = (int)(frame - block.StartFrame);
                        var o = (int)(frame - start) * Channels;
                        mix[o] += block.Lanes[0][at] * volume;
                        mix[o + 1] += block.Lanes[Math.Min(1, block.Lanes.Length - 1)][at] * volume;
                    }
                }
                // What has been handed out has gone to the output: its callback may queue the next.
                for (var i = _blocks.Count - 1; i >= 0; i--)
                {
                    if (_blocks[i].EndFrame <= end)
                    {
                        due.Add(_blocks[i].Rendered);
                        _blocks.RemoveAt(i);
                    }
                }
            }
            _served = end;
        }
        var bytes = new byte[mix.Length * 2];
        for (var i = 0; i < mix.Length; i++)
        {
            var value = (short)Math.Clamp((int)Math.Round(mix[i] * short.MaxValue), short.MinValue, short.MaxValue);
            bytes[2 * i] = (byte)value;
            bytes[2 * i + 1] = (byte)(value >> 8);
        }
        var sample = MediaStreamSample.CreateFromBuffer(
            CryptographicBuffer.CreateFromByteArray(bytes), TimeSpan.FromTicks(start * TimeSpan.TicksPerSecond / Rate));
        sample.Duration = TimeSpan.FromTicks(FramesPerSample * TimeSpan.TicksPerSecond / Rate);
        args.Request.Sample = sample;
        foreach (var rendered in Enumerable.Reverse(due))
        {
            _ui.TryEnqueue(() =>
            {
                if (!_closed)
                {
                    rendered();
                }
            });
        }
    }

    private void OnMediaFailed(MediaPlayer sender, MediaPlayerFailedEventArgs args)
    {
        Interlocked.Exchange(ref _lastFailureTicks, DateTime.UtcNow.Ticks);
        _log.Write($"playback output failed: {args.Error}");
        Changed();
    }

    private void OnDefaultDeviceChanged(object sender, DefaultAudioRenderDeviceChangedEventArgs args)
    {
        if (args.Role == AudioDeviceRole.Default)
        {
            Changed();
        }
    }

    /// <summary>The output can no longer be trusted to play: stopped, then said on the UI thread.</summary>
    private void Changed()
    {
        _ui.TryEnqueue(() =>
        {
            if (_closed)
            {
                return;
            }
            Pause();
            OutputChanged?.Invoke(this, EventArgs.Empty);
        });
    }
}
