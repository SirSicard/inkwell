// The record player (the Mac's RecordPlayerTests): reading the core's chunk files, walking them a
// slice at a time, the two sides on one clock, and the step's check that clicking a chip puts the
// playhead within a second of its stamp. The Mac renders AVAudioEngine offline; here the player
// drives a fake output that mixes what it was given on its own clock, so the same checks run
// headless: what plays at a moment is the samples the player placed there.
using System.Buffers.Binary;
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;

namespace Inkwell.Core.Tests.Screens;

/// <summary>Seconds that pass only as the fake output renders.</summary>
internal sealed class FakePlayerClock
{
    public double Seconds { get; set; }
}

/// <summary>
/// An output that plays nothing aloud: it keeps what the player queued, and <see cref="Render"/>
/// mixes it at 16 kHz on the lanes' clock (each side at its volume), calls back each buffer once
/// it has been rendered, and stops when the device is paused, as the Mac's offline engine does.
/// </summary>
internal sealed class FakeAudioOutput(FakePlayerClock clock) : IRecordAudioOutput
{
    public const int Rate = 16_000;

    private sealed record Block(Channel Side, PcmBuffer Buffer, double At, Action Rendered);

    private readonly List<Block> _blocks = [];
    private readonly Dictionary<Channel, float> _volumes = new() { [Channel.Mic] = 1, [Channel.Far] = 1 };
    private bool _started;
    private bool _running;
    private double _played;

    public bool FailOpen { get; init; }

    public bool Closed { get; private set; }

    public IReadOnlyList<AudioLane> Lanes { get; private set; } = [];

    public event EventHandler? OutputChanged;

    public void Open(IReadOnlyList<AudioLane> lanes)
    {
        if (FailOpen)
        {
            throw new InvalidOperationException("no output device");
        }
        Lanes = lanes;
    }

    public void Schedule(Channel side, PcmBuffer buffer, double atSeconds, Action rendered) =>
        _blocks.Add(new Block(side, buffer, atSeconds, rendered));

    public void Start()
    {
        _started = true;
        _running = true;
        _played = 0;
    }

    public void StopLanes()
    {
        _blocks.Clear();
        _started = false;
    }

    public void Pause() => _running = false;

    public void SetVolume(Channel side, float volume) => _volumes[side] = volume;

    public double? PlayedSeconds => _running && _started ? _played : null;

    public void Close()
    {
        StopLanes();
        _running = false;
        Closed = true;
    }

    /// <summary>The device stops by itself, as one does before a change is announced.</summary>
    public void StopUnderneath() => _running = false;

    /// <summary>The device went away: stopped, then announced.</summary>
    public void Change()
    {
        _running = false;
        OutputChanged?.Invoke(this, EventArgs.Empty);
    }

    /// <summary>Renders up to <paramref name="seconds"/> (less once playback pauses): the mixed samples.</summary>
    public float[] Render(double seconds)
    {
        var output = new List<float>();
        var remaining = (int)Math.Round(seconds * Rate);
        while (remaining > 0 && _running && _started)
        {
            var step = Math.Min(remaining, 1024);
            for (var f = 0; f < step; f++)
            {
                var t = _played + (double)f / Rate;
                var sum = 0f;
                foreach (var block in _blocks)
                {
                    var index = (int)((t - block.At) * block.Buffer.SampleRate);
                    if (t < block.At || index >= block.Buffer.Frames)
                    {
                        continue;
                    }
                    sum += block.Buffer.Channels.Average(c => c[index]) * _volumes[block.Side];
                }
                output.Add(sum);
            }
            _played += (double)step / Rate;
            clock.Seconds += (double)step / Rate;
            remaining -= step;
            // What has gone to the device calls back; the player may queue more, or end.
            var due = _blocks.Where(b => b.At + b.Buffer.Seconds <= _played).ToList();
            foreach (var block in due)
            {
                _blocks.Remove(block);
            }
            foreach (var block in due)
            {
                block.Rendered();
            }
        }
        return output.ToArray();
    }
}

public sealed class RecordPlayerTests : IDisposable
{
    private readonly string _directory = Path.Combine(Path.GetTempPath(), $"inkwell-player-{Guid.NewGuid():N}");
    private readonly FakePlayerClock _clock = new();
    private readonly List<FakeAudioOutput> _outputs = [];
    private readonly Logged _logged = new();

    public RecordPlayerTests()
    {
        Directory.CreateDirectory(_directory);
        // You speak (a tone) from 12.0 to 13.0 s; they speak from 20.0 to 21.0 s. The far end's
        // chunk starts 2 s into the meeting, as a late tap does: its own time places it.
        WriteChunk(PathOf("mic-000000-16000x1.pcm"), Samples(30, 12.0, 13.0));
        WriteChunk(PathOf("far-000000-16000x1.pcm"), Samples(28, 18.0, 19.0));
    }

    public void Dispose()
    {
        try
        {
            Directory.Delete(_directory, recursive: true);
        }
        catch (DirectoryNotFoundException)
        {
        }
    }

    private string PathOf(string name) => Path.Combine(_directory, name);

    private static string Json(string text) => JsonSerializer.Serialize(text, LibraryTestJson.Default.String);

    /// <summary>Writes a chunk file as the core lays one out: a 64-byte header, then little-endian floats.</summary>
    private static void WriteChunk(string path, float[] samples)
    {
        var data = new byte[64 + samples.Length * 4];
        for (var i = 0; i < samples.Length; i++)
        {
            BinaryPrimitives.WriteSingleLittleEndian(data.AsSpan(64 + i * 4), samples[i]);
        }
        File.WriteAllBytes(path, data);
    }

    /// <summary><paramref name="seconds"/> at 16 kHz: silence, with a 440 Hz tone at −10 dBFS from <paramref name="from"/> to <paramref name="to"/>.</summary>
    private static float[] Samples(double seconds, double from, double to) =>
        Enumerable.Range(0, (int)(seconds * 16_000)).Select(i =>
        {
            var t = i / 16_000.0;
            return t >= from && t <= to ? (float)(0.316 * Math.Sin(2 * Math.PI * 440 * t)) : 0f;
        }).ToArray();

    private static float Rms(float[] samples) =>
        samples.Length == 0 ? 0 : MathF.Sqrt(samples.Sum(s => s * s) / samples.Length);

    /// <summary>What plays at the playhead: 0.1 s, after 0.05 s (as the Mac's check, which lets its mixer settle).</summary>
    private static float[] Listen(RecordPlayer player)
    {
        var output = (FakeAudioOutput)player.Output!;
        output.Render(0.05);
        return output.Render(0.1);
    }

    /// <summary>A meeting's library.record answer whose audio is the two chunks above, with a note at 12.4 s (a chip) and a line at 20.1 s.</summary>
    private string Answer(string request, string timeline = "recorded", int leftOut = 0) => $$$"""
        {"type":"library.record","ref":"{{{request}}}",
         "record":{"record":"r1","kind":"meeting","title":"Tones","started_at_unix_ms":0,"ended_at_unix_ms":30000,"revision":2,"has_audio":true},
         "segments":[{"channel":"mic","start_ms":12000,"end_ms":13000,"text":"a tone from you"},
                     {"channel":"far","start_ms":20100,"end_ms":21000,"text":"a tone from them"}],
         "notes":[{"note":"n1","at_ms":12400,"text":"you spoke here"}],
         "commitments":[],"speakers":[],
         "audio":{"timeline":"{{{timeline}}}","left_out":{{{leftOut}}},"chunks":[
           {"channel":"mic","path":{{{Json(PathOf("mic-000000-16000x1.pcm"))}}},"start_ms":0,"frames":480000,"sample_rate":16000,"channels":1,"data_offset":64},
           {"channel":"far","path":{{{Json(PathOf("far-000000-16000x1.pcm"))}}},"start_ms":2000,"frames":448000,"sample_rate":16000,"channels":1,"data_offset":64}]}}
        """;

    private RecordPlayer NewPlayer(RecordDocument document, bool failOpen = false) =>
        new(document, () =>
        {
            var output = new FakeAudioOutput(_clock) { FailOpen = failOpen };
            _outputs.Add(output);
            return output;
        }, () => _clock.Seconds, _logged.Log);

    /// <summary>A library with the record open, its player on a fake output.</summary>
    private LibraryModel OpenRecord(string timeline = "recorded", int leftOut = 0)
    {
        var sent = new Sent();
        var library = new LibraryModel(sent.Send, makePlayer: d => NewPlayer(d));
        library.Open("r1");
        library.Apply(Ev.Of(Answer(LibraryFixtures.RequestId(sent.Commands[^1]), timeline, leftOut)));
        Assert.NotNull(library.Player);
        return library;
    }

    /// <summary>The step's check: a chip's click puts the playhead within a second of its stamp, and what plays there is the audio at that moment.</summary>
    [Fact]
    public void ClickingAChipPutsThePlayheadAtItsStamp()
    {
        var library = OpenRecord();
        var chip = library.Document!.Merged.First(e => e.Kind == MergedKind.Note);
        Assert.Equal(12_400, chip.AtMs);

        library.PlayFrom(chip.AtMs); // what the chip's button does
        var player = library.Player!;
        Assert.Equal(PlayerState.Playing, player.State);
        var heard = Listen(player);
        var position = player.PositionMs();
        Assert.True(Math.Abs(position - chip.AtMs) <= 1_000, $"playhead at {position} ms");
        Assert.True(position > chip.AtMs - 1, "it moved on from the stamp");
        Assert.True(Rms(heard) > 0.05, "your tone plays at 12.4 s");
        Assert.Equal(0, library.PlayheadLine()); // the ledger marks your line

        // A moment where nobody speaks: silence plays there.
        library.PlayFrom(5_000);
        var quiet = Listen(player);
        Assert.True(Math.Abs(player.PositionMs() - 5_000) <= 1_000);
        Assert.True(Rms(quiet) < 0.001);
        player.Stop();
    }

    /// <summary>The two sides share one clock: the far end's chunk, which started 2 s late, plays at its own place; a line's click plays from it; and the mix silences a side.</summary>
    [Fact]
    public void BothSidesShareTheTimelineAndTheMixSilencesASide()
    {
        var library = OpenRecord();
        var player = library.Player!;
        var line = library.Document!.Ledger.First(l => !l.Speaker.IsYou);
        Assert.Equal(20_100, line.StartMs);

        library.PlayFrom(line.StartMs); // what the ledger row's button does
        Assert.True(Rms(Listen(player)) > 0.05, "their tone, 18 s into a chunk that starts at 2 s, plays at 20.1 s");
        Assert.True(Math.Abs(player.PositionMs() - line.StartMs) <= 1_000);

        player.ThemVolume = 0;
        player.Seek(line.StartMs);
        Assert.True(Rms(Listen(player)) < 0.001, "their side muted");

        player.YouVolume = 0;
        player.ThemVolume = 1;
        player.Seek(12_400);
        Assert.True(Rms(Listen(player)) < 0.001, "your side muted");
        player.Stop();
    }

    /// <summary>Playing on across a slice boundary keeps going (the next slice is queued as one finishes), and the end of the audio ends playback.</summary>
    [Fact]
    public void PlaybackRunsAcrossSlicesToTheEnd()
    {
        var library = OpenRecord();
        var player = library.Player!;
        library.PlayFrom(21_000);
        // 9 s to the end of the longest side: more than two slices of 4 s.
        for (var i = 0; i < 12 && player.State != PlayerState.Ended; i++)
        {
            ((FakeAudioOutput)player.Output!).Render(1);
        }
        Assert.Equal(PlayerState.Ended, player.State);
        Assert.Equal(player.DurationMs, player.PositionMs());
        player.Stop();
    }

    /// <summary>Windows addition: playing from the very end ends at once (nothing is left to queue), where the Mac's player would play silence.</summary>
    [Fact]
    public void PlayingFromTheVeryEndEndsAtOnce()
    {
        var library = OpenRecord();
        var player = library.Player!;
        library.PlayFrom(player.DurationMs);
        Assert.Equal(PlayerState.Ended, player.State);
        player.Play();
        Assert.Equal(PlayerState.Playing, player.State); // Play after the end starts again from the top
        Assert.Equal(0, player.AnchorMs);
        player.Stop();
    }

    [Fact]
    public void TheWaveformHasOneLanePerSideAndPeaksWhereEachSpoke()
    {
        var doc = OpenRecord().Document!;
        var wave = Waveform.Build(doc.Chunks, 30_000, 30);
        Assert.Equal(30, wave.You.Count);
        Assert.Equal(12, wave.You.ToList().FindIndex(v => v > 0.5));
        Assert.Equal(20, wave.Them.ToList().FindIndex(v => v > 0.5)); // placed by its chunk's start
        Assert.Equal(0, wave.You[5]);
    }

    [Fact]
    public void ASliceCursorWalksChunksFromAMomentAndSkipsGaps()
    {
        var a = new TimelineChunk(Channel.Mic, "a", 0, 16_000, 16_000, 1, 64);
        var b = new TimelineChunk(Channel.Mic, "b", 5_000, 32_000, 16_000, 1, 64);
        var cursor = new SliceCursor([b, a], 500);
        Assert.Equal(new ChunkSlice(a, 8_000, 8_000), cursor.Next(4));
        Assert.Equal(new ChunkSlice(b, 0, 24_000), cursor.Next(1.5));
        Assert.Equal(6.5, cursor.Next(4)?.StartSeconds);
        Assert.Null(cursor.Next(4));
        var inGap = new SliceCursor([a, b], 2_000);
        Assert.Equal("b", inGap.Next(1)?.Chunk.Path); // a moment in a gap starts at the next chunk
    }

    [Fact]
    public void AChunkReadsAsItsFramesDeinterleaved()
    {
        var path = PathOf("far-000001-8000x2.pcm");
        WriteChunk(path, [0.1f, -0.1f, 0.2f, -0.2f, 0.3f, -0.3f]);
        var chunk = new TimelineChunk(Channel.Far, path, 0, 3, 8_000, 2, 64);
        var buffer = ChunkAudio.Buffer(new ChunkSlice(chunk, 1, 2));
        Assert.Equal(2, buffer.Frames);
        Assert.Equal(2, buffer.Channels.Count);
        Assert.Equal(0.2f, buffer.Channels[0][0]);
        Assert.Equal(-0.3f, buffer.Channels[1][1]);
        var missing = chunk with { Path = PathOf("gone.pcm") };
        Assert.Throws<ChunkReadException>(() => ChunkAudio.Buffer(new ChunkSlice(missing, 0, 1)));
    }

    // Review fixes

    /// <summary>Item 1: an estimated timeline (the meeting's start was not written) and chunks left out reach what the Record screen reads, with the words it shows; a precise one shows none.</summary>
    [Fact]
    public void AnEstimatedTimelineAndMissingAudioReachTheScreensState()
    {
        var precise = OpenRecord().Document!;
        Assert.Empty(precise.PlaybackCaveats.Messages(waveformPartial: false));
        Assert.Equal("Blotted · final", precise.LedgerStatus(blottedAt: null));

        var library = OpenRecord(timeline: "estimated", leftOut: 2);
        var doc = library.Document!;
        Assert.True(doc.PlaybackCaveats.TimelineEstimated);
        Assert.Equal(2, doc.PlaybackCaveats.LeftOut);
        Assert.Equal(
            ["Timing estimated: the two sides may be out of step.", "Part of this recording can't be played."],
            doc.PlaybackCaveats.Messages(waveformPartial: false));
        Assert.Equal("Blotted · final · timing estimated", doc.LedgerStatus(blottedAt: null));
        Assert.Equal("Part of this recording can't be played.", doc.PlaybackCaveats.Messages(waveformPartial: true)[^1]); // said once
        Assert.Equal(["Part of this recording can't be played."], new PlaybackCaveats(false, 0).Messages(waveformPartial: true));
    }

    /// <summary>Item 5: a chunk shorter on disk than its frame count is an error, never fewer samples with the cursor moving on by the full count (which would drift the rest of the side).</summary>
    [Fact]
    public void ATruncatedChunkIsAnErrorNotDrift()
    {
        var path = PathOf("mic-000009-16000x1.pcm");
        WriteChunk(path, Enumerable.Repeat(0.1f, 50).ToArray());
        var chunk = new TimelineChunk(Channel.Mic, path, 0, 100, 16_000, 1, 64);
        Assert.Equal(50, ChunkAudio.Samples(new ChunkSlice(chunk, 0, 50)).Length);
        var error = Assert.Throws<ChunkReadException>(() => ChunkAudio.Samples(new ChunkSlice(chunk, 0, 100)));
        Assert.Equal(ChunkReadError.Truncated, error.Error);
        Assert.Throws<ChunkReadException>(() => ChunkAudio.Samples(new ChunkSlice(chunk, 60, 10)));
    }

    /// <summary>Item 3: a chunk that cannot be read leaves its stretch flat, and the waveform says it is partial (the player bar shows that), rather than looking like silence.</summary>
    [Fact]
    public void AWaveformWithAnUnreadableChunkIsMarkedPartial()
    {
        var doc = OpenRecord().Document!;
        Assert.False(Waveform.Build(doc.Chunks, 30_000, 30).Partial);
        var chunks = doc.Chunks.Append(new TimelineChunk(Channel.Far, PathOf("far-000001-16000x1.pcm"), 28_000, 16_000, 16_000, 1, 64)).ToList();
        var partial = Waveform.Build(chunks, 30_000, 30, log: _logged.Log);
        Assert.True(partial.Partial);
        Assert.Equal(12, partial.You.ToList().FindIndex(v => v > 0.5)); // what could be read is still drawn
        Assert.Equal(["waveform: a far chunk could not be read (Unreadable); drawn flat"], _logged.Messages);
    }

    /// <summary>Item 6: switching records quickly never shows the first record's waveform over the second's: the first build is cancelled, and a late result is checked against the record shown.</summary>
    [Fact]
    public async Task ARapidSwitchKeepsTheNewestRecordsWaveform()
    {
        var longer = OpenRecord().Document!;
        var shortPath = PathOf("mic-000000-16000x1-short.pcm");
        WriteChunk(shortPath, Samples(2, 0.5, 1.0));
        var shorter = new RecordDocument(Ev.Of<LibraryRecord>($$$"""
            {"type":"library.record","ref":"x","record":{"record":"r2","kind":"meeting","started_at_unix_ms":0,"ended_at_unix_ms":2000,"revision":2,"has_audio":true},
             "segments":[],"notes":[],"commitments":[],"speakers":[],
             "audio":{"timeline":"recorded","left_out":0,"chunks":[{"channel":"mic","path":{{{Json(shortPath)}}},"start_ms":0,"frames":32000,"sample_rate":16000,"channels":1,"data_offset":64}]}}
            """));
        var loader = new WaveformLoader(_logged.Log);
        loader.Load(longer, 40);
        loader.Load(shorter, 40);
        await loader.Settled();
        Assert.Equal("r2", loader.Record);
        Assert.Equal(Waveform.Build(shorter.Chunks, shorter.DurationMs, 40), loader.Waveform);
        // And the other way round.
        loader.Load(shorter, 40);
        loader.Load(longer, 40);
        await loader.Settled();
        Assert.Equal("r1", loader.Record);
        Assert.Equal(Waveform.Build(longer.Chunks, longer.DurationMs, 40), loader.Waveform);
    }

    /// <summary>Item 7: the output changing under a playing player (a device unplugged, the default moved) restarts playback in place, from where it was, rather than going silent unnoticed.</summary>
    [Fact]
    public void AnOutputChangeRestartsPlaybackInPlace()
    {
        var library = OpenRecord();
        var player = library.Player!;
        library.PlayFrom(12_400);
        Listen(player);
        var before = player.PositionMs();
        var output = (FakeAudioOutput)player.Output!;
        output.Change();
        Assert.Equal(PlayerState.Playing, player.State);
        Assert.NotSame(output, player.Output); // a new output
        Assert.True(output.Closed);
        var heard = Listen(player);
        Assert.True(Math.Abs(player.PositionMs() - before) <= 1_000, "from where it was");
        Assert.True(Rms(heard) > 0.05, "and it plays");

        // Paused, a change only lets the output go; Play opens a new one.
        player.Pause();
        var paused = (FakeAudioOutput)player.Output!;
        paused.Change();
        Assert.Equal(PlayerState.Paused, player.State);
        Assert.Null(player.Output);
        player.Stop();
    }

    /// <summary>
    /// Re-check fix: a device stops before its change is announced, and a stopped device has no
    /// clock. The player keeps its own clock of the run, so it resumes from where the listener
    /// was, not from the last play or seek.
    /// </summary>
    [Fact]
    public void AnOutputChangeAfterTheEngineStoppedResumesWhereItWasNotAtTheAnchor()
    {
        var library = OpenRecord();
        var player = library.Player!;
        library.PlayFrom(5_000);
        var output = (FakeAudioOutput)player.Output!;
        for (var i = 0; i < 4; i++)
        {
            output.Render(1); // 4 s
        }
        var before = player.PositionMs();
        Assert.True(before > 8_500, "well past the anchor");
        output.StopUnderneath();
        Assert.Equal(before, player.PositionMs()); // a stopped device does not lose the position
        output.Change();
        Assert.Equal(PlayerState.Playing, player.State);
        Assert.True(Math.Abs(player.AnchorMs - before) <= 500, $"resumed at {player.AnchorMs}, was at {before}");
        player.Stop();
    }

    /// <summary>Item 7: when playback cannot start again after an output change, it says so (Failed with its words), never plays nothing as if all were well.</summary>
    [Fact]
    public void AnOutputChangeThatCannotRestartFailsVisibly()
    {
        var library = OpenRecord();
        var player = library.Player!;
        library.PlayFrom(12_400);
        Listen(player);
        var output = (FakeAudioOutput)player.Output!;
        // The files go before the change: the new output cannot be fed.
        Directory.Delete(_directory, recursive: true);
        output.Change();
        Assert.Equal(PlayerState.Failed, player.State);
        Assert.Equal("This recording can't be played right now.", player.Failure);
        Assert.Equal("This recording can't be played right now.", player.Notice(library.Document!.PlaybackCaveats, waveformPartial: false));
        Assert.Equal(["playback failed: chunk Unreadable"], _logged.Messages);
    }

    /// <summary>Windows addition: no output device at Play is a failure the player bar shows, logged by kind only.</summary>
    [Fact]
    public void NoOutputDeviceFailsVisibly()
    {
        var sent = new Sent();
        var library = new LibraryModel(sent.Send, makePlayer: d => NewPlayer(d, failOpen: true));
        library.Open("r1");
        library.Apply(Ev.Of(Answer(LibraryFixtures.RequestId(sent.Commands[^1]))));
        library.PlayFrom(12_400);
        Assert.Equal(PlayerState.Failed, library.Player!.State);
        Assert.Null(library.Player.Output);
        Assert.Equal(["playback failed: InvalidOperationException"], _logged.Messages);
    }
}

[System.Text.Json.Serialization.JsonSerializable(typeof(string))]
internal sealed partial class LibraryTestJson : System.Text.Json.Serialization.JsonSerializerContext;
