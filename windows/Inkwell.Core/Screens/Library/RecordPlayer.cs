// Plays a record, as the Mac's RecordPlayer: one lane per side (You, the mic; Them, the far end),
// both on the output's clock and started at the same moment, each with its own volume (the
// You/Them mix). Audio stays on disk: each side keeps two slices of a few seconds queued, and
// queues the next as one finishes. Every slice is placed by its own time on the record's
// timeline, so the sides stay aligned across gaps, and a line's stamp and the audio agree.
//
// The sound itself goes through IRecordAudioOutput, which the app implements over the Windows
// audio stack and the tests fake, so this logic runs headless. The output exists only while
// something plays or is paused: an idle Record screen holds no audio device and draws nothing
// (architecture rule 9).
//
// When the output changes under it (a device unplugged, the default device moved: the output's
// OutputChanged), the output is let go and playback starts again in place, from where it was; if
// it cannot, the player fails with its words rather than going silent.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>One side's lane on the output, in the format its buffers come in.</summary>
public readonly record struct AudioLane(Channel Side, int SampleRate, int Channels);

/// <summary>
/// Where a record's sound goes. The app's implementation (over WASAPI or AudioGraph) must:
/// <list type="bullet">
/// <item>play every lane in step: buffers are placed by their offset from one start moment shared by the lanes, sample-aligned, with silence between them;</item>
/// <item>mix the lanes to the default output device at each lane's volume;</item>
/// <item>call a buffer's <c>rendered</c> callback once, on the UI thread, when that buffer has gone to the device, and never after <see cref="StopLanes"/> or <see cref="Close"/>;</item>
/// <item>raise <see cref="OutputChanged"/> on the UI thread when the device goes or the default moves (it has stopped by then);</item>
/// <item>throw from <see cref="Open"/> or <see cref="Start"/> when it cannot play (no device): the player then says so.</item>
/// </list>
/// Buffers arrive in their lane's format (the player converts); the output never reads files.
/// </summary>
public interface IRecordAudioOutput
{
    /// <summary>Opens the device with one lane per side.</summary>
    void Open(IReadOnlyList<AudioLane> lanes);

    /// <summary>Queues <paramref name="buffer"/> on <paramref name="side"/>'s lane, <paramref name="atSeconds"/> after the lanes start.</summary>
    void Schedule(Channel side, PcmBuffer buffer, double atSeconds, Action rendered);

    /// <summary>Starts (or resumes) the device and every lane at one moment; the lanes' clock starts at zero.</summary>
    void Start();

    /// <summary>Stops every lane and drops what is queued.</summary>
    void StopLanes();

    /// <summary>Pauses the device (its lanes are stopped).</summary>
    void Pause();

    void SetVolume(Channel side, float volume);

    /// <summary>Seconds the lanes have played since <see cref="Start"/>, on the device's clock; null while it is not running.</summary>
    double? PlayedSeconds { get; }

    /// <summary>Lets go of the device.</summary>
    void Close();

    /// <summary>The device went away or the default output moved; raised on the UI thread.</summary>
    event EventHandler? OutputChanged;
}

public enum PlayerState
{
    Idle,
    Playing,
    Paused,
    Ended,
    /// <summary>The audio could not play (no output device, unreadable files): <see cref="RecordPlayer.Failure"/> says so on screen.</summary>
    Failed,
}

public sealed class RecordPlayer : ObservableModel
{
    /// <summary>How much of each side is queued ahead: two slices of this many seconds.</summary>
    public const double SliceSeconds = 4.0;
    public const int SlicesAhead = 2;

    /// <summary>What the player bar says when the audio cannot play.</summary>
    public const string FailureText = "This recording can't be played right now.";

    private readonly Dictionary<Channel, List<TimelineChunk>> _chunks;
    private readonly Dictionary<Channel, AudioLane> _lanes = [];
    private readonly Func<IRecordAudioOutput> _makeOutput;
    private readonly Func<double> _clock;
    private readonly ScreenLog _log;
    private IRecordAudioOutput? _output;
    private readonly Dictionary<Channel, SliceCursor> _cursors = [];
    private readonly Dictionary<Channel, int> _queued = [];
    private readonly HashSet<Channel> _exhausted = [];
    /// <summary>The run's own clock (seconds on <see cref="_clock"/> when the lanes started), which an output stopped under it does not take away.</summary>
    private double? _runStart;
    /// <summary>The furthest point on the timeline queued so far: the run's clock never reads past it.</summary>
    private long _scheduledEndMs;
    /// <summary>Raised by every seek, pause and stop: a completion from an earlier run changes nothing.</summary>
    private int _run;
    private float _youVolume = 1;
    private float _themVolume = 1;

    /// <param name="makeOutput">A new output: once per play from idle, and again after the output changes.</param>
    /// <param name="clock">Monotonic seconds, for the run's own clock (defaults to Stopwatch).</param>
    public RecordPlayer(RecordDocument document, Func<IRecordAudioOutput> makeOutput, Func<double>? clock = null, ScreenLog? log = null)
    {
        ArgumentNullException.ThrowIfNull(document);
        ArgumentNullException.ThrowIfNull(makeOutput);
        _chunks = document.Chunks.GroupBy(c => c.Channel).ToDictionary(g => g.Key, g => g.ToList());
        Sides = new[] { Channel.Mic, Channel.Far }.Where(_chunks.ContainsKey).ToList();
        foreach (var side in Sides)
        {
            var first = _chunks[side][0];
            _lanes[side] = new AudioLane(side, Math.Max(first.SampleRate, 1), Math.Max(first.Channels, 1));
        }
        DurationMs = document.DurationMs;
        _makeOutput = makeOutput;
        _clock = clock ?? (() => System.Diagnostics.Stopwatch.GetTimestamp() / (double)System.Diagnostics.Stopwatch.Frequency);
        _log = log ?? ScreenLog.System;
    }

    public PlayerState State { get; private set; } = PlayerState.Idle;

    /// <summary>Why it cannot play, when <see cref="State"/> is <see cref="PlayerState.Failed"/>.</summary>
    public string? Failure => State == PlayerState.Failed ? FailureText : null;

    /// <summary>Where the current run started (or where a paused one stopped), ms on the record's timeline.</summary>
    public long AnchorMs { get; private set; }

    public long DurationMs { get; }

    /// <summary>The sides this record has audio for.</summary>
    public IReadOnlyList<Channel> Sides { get; }

    public bool IsPlaying => State == PlayerState.Playing;

    /// <summary>Your side's volume, 0 to 1.</summary>
    public float YouVolume
    {
        get => _youVolume;
        set
        {
            _youVolume = Math.Clamp(value, 0, 1);
            _output?.SetVolume(Channel.Mic, _youVolume);
            Changed();
        }
    }

    /// <summary>Their side's volume, 0 to 1.</summary>
    public float ThemVolume
    {
        get => _themVolume;
        set
        {
            _themVolume = Math.Clamp(value, 0, 1);
            _output?.SetVolume(Channel.Far, _themVolume);
            Changed();
        }
    }

    /// <summary>The output now (tests: to change it under the player).</summary>
    internal IRecordAudioOutput? Output => _output;

    // Control

    /// <summary>Plays from the playhead.</summary>
    public void Play()
    {
        if (Sides.Count == 0 || State == PlayerState.Playing)
        {
            return;
        }
        if (State == PlayerState.Ended)
        {
            AnchorMs = 0;
        }
        try
        {
            StartOutput();
            StartRun();
            State = PlayerState.Playing;
            Changed();
            EndIfNothingQueued();
        }
        catch (Exception e) when (e is not OutOfMemoryException)
        {
            Fail(e);
        }
    }

    /// <summary>Stops where it is; play goes on from there.</summary>
    public void Pause()
    {
        if (State != PlayerState.Playing)
        {
            return;
        }
        AnchorMs = PositionMs();
        StopLanes();
        _output?.Pause();
        State = PlayerState.Paused;
        Changed();
    }

    /// <summary>Plays or pauses.</summary>
    public void Toggle()
    {
        if (IsPlaying)
        {
            Pause();
        }
        else
        {
            Play();
        }
    }

    /// <summary>Moves the playhead to <paramref name="ms"/>, playing on from there if it was playing.</summary>
    public void Seek(long ms)
    {
        var target = Math.Clamp(ms, 0, DurationMs);
        var wasPlaying = State == PlayerState.Playing;
        StopLanes();
        AnchorMs = target;
        if (wasPlaying)
        {
            try
            {
                StartRun();
                EndIfNothingQueued();
            }
            catch (Exception e) when (e is not OutOfMemoryException)
            {
                Fail(e);
                return;
            }
        }
        else if (State == PlayerState.Ended)
        {
            State = PlayerState.Paused;
        }
        Changed();
    }

    /// <summary>Lets go of the output.</summary>
    public void Stop()
    {
        ReleaseOutput();
        if (State is PlayerState.Playing or PlayerState.Paused or PlayerState.Ended)
        {
            State = PlayerState.Idle;
        }
        Changed();
    }

    /// <summary>
    /// Where the playhead is now, ms on the record's timeline. Read while drawing (the view
    /// redraws only while playing). From the output's clock while it runs, else from the run's own
    /// clock (the output stopped under it: a device change).
    /// </summary>
    public long PositionMs()
    {
        if (State != PlayerState.Playing)
        {
            return AnchorMs;
        }
        if (_output?.PlayedSeconds is double played)
        {
            return Math.Min(AnchorMs + (long)(Math.Max(played, 0) * 1000), DurationMs);
        }
        if (_runStart is not double start)
        {
            return AnchorMs;
        }
        var elapsed = Math.Max(_clock() - start, 0);
        var reached = Math.Min(Math.Max(_scheduledEndMs, AnchorMs), DurationMs);
        return Math.Min(AnchorMs + (long)(elapsed * 1000), reached);
    }

    /// <summary>Whether the playhead has been put somewhere: by playing, or by a chip or a line (the ledger marks a line only then).</summary>
    public bool IsPlaced => State != PlayerState.Idle || AnchorMs > 0;

    /// <summary>The player bar's time: <c>12:41 / 30:00</c>.</summary>
    public string TimeText(long positionMs) => $"{LibraryFormat.Stamp(positionMs)} / {LibraryFormat.Stamp(DurationMs)}";

    /// <summary>The player bar's time as a screen reader reads it.</summary>
    public string TimeLabel(long positionMs) => $"{LibraryFormat.Stamp(positionMs)} of {LibraryFormat.Stamp(DurationMs)}";

    /// <summary>The play button's name.</summary>
    public string PlayLabel => IsPlaying ? "Pause" : "Play";

    /// <summary>Where a click at <paramref name="fraction"/> of the waveform's width puts the playhead.</summary>
    public long MsAt(double fraction) => (long)(Math.Clamp(fraction, 0, 1) * DurationMs);

    /// <summary>What the player bar says under the waveform: the failure, then what it cannot vouch for; null when all is well.</summary>
    public string? Notice(PlaybackCaveats caveats, bool waveformPartial)
    {
        ArgumentNullException.ThrowIfNull(caveats);
        var lines = new List<string>();
        if (Failure is string failure)
        {
            lines.Add(failure);
        }
        lines.AddRange(caveats.Messages(waveformPartial));
        return lines.Count > 0 ? string.Join(" ", lines) : null;
    }

    // The output

    private void StartOutput()
    {
        if (_output is not null)
        {
            return;
        }
        var output = _makeOutput();
        try
        {
            output.Open(Sides.Select(s => _lanes[s]).ToList());
            output.SetVolume(Channel.Mic, _youVolume);
            output.SetVolume(Channel.Far, _themVolume);
        }
        catch
        {
            output.Close();
            throw;
        }
        output.OutputChanged += OnOutputChanged;
        _output = output;
    }

    /// <summary>Stops and drops the output, and stops listening to it.</summary>
    private void ReleaseOutput()
    {
        StopLanes();
        if (_output is { } output)
        {
            output.OutputChanged -= OnOutputChanged;
            output.Close();
        }
        _output = null;
    }

    /// <summary>The output changed: it has stopped. A new one starts from where playback was; paused, the next Play starts it.</summary>
    private void OnOutputChanged(object? sender, EventArgs e)
    {
        if (!ReferenceEquals(sender, _output))
        {
            return;
        }
        var wasPlaying = State == PlayerState.Playing;
        var at = PositionMs();
        ReleaseOutput();
        AnchorMs = at;
        if (!wasPlaying)
        {
            Changed();
            return;
        }
        try
        {
            StartOutput();
            StartRun();
            State = PlayerState.Playing;
            Changed();
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            Fail(ex);
        }
    }

    /// <summary>Queues each side from the anchor and starts both lanes at one moment.</summary>
    private void StartRun()
    {
        _run++;
        _exhausted.Clear();
        _scheduledEndMs = AnchorMs;
        foreach (var side in Sides)
        {
            _cursors[side] = new SliceCursor(_chunks[side], AnchorMs);
            _queued[side] = 0;
            Fill(side);
        }
        _output?.Start();
        _runStart = _clock();
    }

    /// <summary>Keeps <see cref="SlicesAhead"/> slices of <paramref name="side"/> queued.</summary>
    private void Fill(Channel side)
    {
        if (_output is not { } output)
        {
            return;
        }
        var lane = _lanes[side];
        while (_queued.GetValueOrDefault(side) < SlicesAhead)
        {
            if (_cursors[side].Next(SliceSeconds) is not { } slice)
            {
                _exhausted.Add(side);
                break;
            }
            var buffer = ChunkAudio.Convert(ChunkAudio.Buffer(slice), lane.SampleRate, lane.Channels);
            // Its place on the lanes' clock, which starts at the anchor.
            var at = Math.Max(slice.StartSeconds - AnchorMs / 1000.0, 0);
            _scheduledEndMs = Math.Max(_scheduledEndMs, slice.EndMs);
            var run = _run;
            _queued[side] = _queued.GetValueOrDefault(side) + 1;
            output.Schedule(side, buffer, at, () => Played(side, run));
        }
    }

    private void Played(Channel side, int run)
    {
        if (run != _run || State != PlayerState.Playing)
        {
            return;
        }
        _queued[side] = Math.Max(_queued.GetValueOrDefault(side, 1) - 1, 0);
        try
        {
            Fill(side);
        }
        catch (ChunkReadException e)
        {
            Fail(e);
            return;
        }
        EndIfNothingQueued();
    }

    /// <summary>
    /// Every side has run out and nothing is queued: playback has ended. (Also when a run starts
    /// with nothing left to play, at the very end, which the Mac's player leaves playing silence.)
    /// </summary>
    private void EndIfNothingQueued()
    {
        if (State != PlayerState.Playing || !Sides.All(s => _exhausted.Contains(s) && _queued.GetValueOrDefault(s) == 0))
        {
            return;
        }
        StopLanes();
        _output?.Pause();
        AnchorMs = DurationMs;
        State = PlayerState.Ended;
        Changed();
    }

    private void StopLanes()
    {
        // The new run number ignores any completion of what was queued.
        _run++;
        _output?.StopLanes();
        _queued.Clear();
        _runStart = null;
    }

    private void Fail(Exception error)
    {
        // The kind of failure, never a path or what was said.
        var kind = error is ChunkReadException read ? $"chunk {read.Error}" : error.GetType().Name;
        _log.Write($"playback failed: {kind}");
        ReleaseOutput();
        State = PlayerState.Failed;
        Changed();
    }
}
