// The record player: reading the core's chunk files, walking them a slice at a time, the two
// sides on one clock, and the step's check that clicking a chip puts the playhead within a second
// of its stamp. Rendered offline (AVAudioEngine's manual rendering), so no audio device is needed
// and CI runs it too.
import AVFoundation
import Foundation
import InkBridge
import XCTest

@testable import Inkwell

/// Writes a chunk file as the core lays one out: a 64-byte header, then little-endian floats.
/// The player reads only past `data_offset`, so the header's bytes are left zero here.
private func writeChunk(_ url: URL, samples: [Float]) throws {
    var data = Data(count: 64)
    for s in samples {
        withUnsafeBytes(of: s.bitPattern.littleEndian) { data.append(contentsOf: $0) }
    }
    try data.write(to: url)
}

/// `seconds` at 16 kHz: silence, with a 440 Hz tone at −10 dBFS over `tone`.
private func samples(seconds: Double, tone: ClosedRange<Double>) -> [Float] {
    let rate = 16_000.0
    return (0..<Int(seconds * rate)).map { i in
        let t = Double(i) / rate
        return tone.contains(t) ? Float(0.316 * sin(2 * .pi * 440 * t)) : 0
    }
}

private func rms(_ buffer: AVAudioPCMBuffer?) -> Float {
    guard let buffer, let data = buffer.floatChannelData, buffer.frameLength > 0 else { return 0 }
    var sum: Float = 0
    for c in 0..<Int(buffer.format.channelCount) {
        for f in 0..<Int(buffer.frameLength) {
            sum += data[c][f] * data[c][f]
        }
    }
    return (sum / Float(Int(buffer.frameLength) * Int(buffer.format.channelCount))).squareRoot()
}

/// What plays at the playhead: 0.1 s, after 0.05 s for the mixer's rate converter and volume
/// ramp to settle (a change of place or volume fades in over a few milliseconds).
@MainActor
private func listen(_ player: RecordPlayer) async throws -> AVAudioPCMBuffer? {
    _ = try await player.renderOffline(frames: 2_400)
    return try await player.renderOffline(frames: 4_800)
}

@MainActor
final class RecordPlayerTests: XCTestCase {
    private var directory: URL!

    override func setUp() async throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-player-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // You speak (a tone) from 12.0 to 13.0 s; they speak from 20.0 to 21.0 s. The far end's
        // chunk starts 2 s into the meeting, as a late tap does: its own time places it.
        try writeChunk(directory.appendingPathComponent("mic-000000-16000x1.pcm"), samples: samples(seconds: 30, tone: 12.0...13.0))
        try writeChunk(directory.appendingPathComponent("far-000000-16000x1.pcm"), samples: samples(seconds: 28, tone: 18.0...19.0))
    }

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: directory)
    }

    /// A meeting's `library.record` answer whose audio is the two chunks above, with a note at
    /// 12.4 s (a chip) and a line at 20.1 s.
    private func answer(request: String, timeline: String = "recorded", leftOut: Int = 0) -> String {
        let mic = directory.appendingPathComponent("mic-000000-16000x1.pcm").path
        let far = directory.appendingPathComponent("far-000000-16000x1.pcm").path
        return #"""
        {"type":"library.record","ref":"\#(request)",
         "record":{"record":"r1","kind":"meeting","title":"Tones","started_at_unix_ms":0,"ended_at_unix_ms":30000,"revision":2,"has_audio":true},
         "segments":[{"channel":"mic","start_ms":12000,"end_ms":13000,"text":"a tone from you"},
                     {"channel":"far","start_ms":20100,"end_ms":21000,"text":"a tone from them"}],
         "notes":[{"note":"n1","at_ms":12400,"text":"you spoke here"}],
         "commitments":[],"speakers":[],
         "audio":{"timeline":"\#(timeline)","left_out":\#(leftOut),"chunks":[
           {"channel":"mic","path":"\#(mic)","start_ms":0,"frames":480000,"sample_rate":16000,"channels":1,"data_offset":64},
           {"channel":"far","path":"\#(far)","start_ms":2000,"frames":448000,"sample_rate":16000,"channels":1,"data_offset":64}]}}
        """#
    }

    /// A library with the record open, its player rendering offline.
    private func openRecord(timeline: String = "recorded", leftOut: Int = 0) throws -> LibraryModel {
        var sent: [String] = []
        let library = LibraryModel(send: { sent.append($0.json) })
        library.makePlayer = { RecordPlayer(document: $0, output: .offline(sampleRate: 48_000, channels: 2)) }
        library.open("r1")
        let id = ((try JSONSerialization.jsonObject(with: Data(sent.last!.utf8))) as? [String: Any])?["id"] as? String ?? ""
        library.apply([try InkEvent.decode(Data(answer(request: id, timeline: timeline, leftOut: leftOut).utf8))])
        XCTAssertNotNil(library.player)
        return library
    }

    /// The step's check: a chip's click puts the playhead within a second of its stamp, and what
    /// plays there is the audio at that moment.
    func testClickingAChipPutsThePlayheadAtItsStamp() async throws {
        let library = try openRecord()
        let chip = try XCTUnwrap(library.document?.merged.first { $0.kind == .note })
        XCTAssertEqual(chip.atMs, 12_400)

        library.playFrom(chip.atMs)  // what the chip's button does
        let player = try XCTUnwrap(library.player)
        XCTAssertEqual(player.state, .playing)
        let heard = try await listen(player)
        let position = player.positionMs()
        XCTAssertLessThanOrEqual(abs(position - chip.atMs), 1_000, "playhead at \(position) ms")
        XCTAssertGreaterThan(position, chip.atMs - 1, "it moved on from the stamp")
        XCTAssertGreaterThan(rms(heard), 0.05, "your tone plays at 12.4 s")

        // A moment where nobody speaks: silence plays there.
        library.playFrom(5_000)
        let quiet = try await listen(player)
        XCTAssertLessThanOrEqual(abs(player.positionMs() - 5_000), 1_000)
        XCTAssertLessThan(rms(quiet), 0.001)
        player.stop()
    }

    /// The two sides share one clock: the far end's chunk, which started 2 s late, plays at its
    /// own place; a line's click plays from it; and the mix silences a side.
    func testBothSidesShareTheTimelineAndTheMixSilencesASide() async throws {
        let library = try openRecord()
        let player = try XCTUnwrap(library.player)
        let line = try XCTUnwrap(library.document?.ledger.first { !$0.speaker.isYou })
        XCTAssertEqual(line.startMs, 20_100)

        library.playFrom(line.startMs)  // what the ledger row's button does
        let theirs = try await listen(player)
        XCTAssertGreaterThan(rms(theirs), 0.05, "their tone, 18 s into a chunk that starts at 2 s, plays at 20.1 s")
        XCTAssertLessThanOrEqual(abs(player.positionMs() - line.startMs), 1_000)

        player.themVolume = 0
        player.seek(toMs: line.startMs)
        let muted = try await listen(player)
        XCTAssertLessThan(rms(muted), 0.001, "their side muted")

        player.youVolume = 0
        player.themVolume = 1
        player.seek(toMs: 12_400)
        let yoursMuted = try await listen(player)
        XCTAssertLessThan(rms(yoursMuted), 0.001, "your side muted")
        player.stop()
    }

    /// Playing on across a slice boundary keeps going (the next slice is queued as one finishes),
    /// and the end of the audio ends playback.
    func testPlaybackRunsAcrossSlicesToTheEnd() async throws {
        let library = try openRecord()
        let player = try XCTUnwrap(library.player)
        library.playFrom(21_000)
        // 9 s to the end of the longest side: more than two slices of 4 s.
        for _ in 0..<12 {
            _ = try await player.renderOffline(frames: 48_000)
            if player.state == .ended { break }
        }
        XCTAssertEqual(player.state, .ended)
        XCTAssertEqual(player.positionMs(), player.durationMs)
        player.stop()
    }

    func testTheWaveformHasOneLanePerSideAndPeaksWhereEachSpoke() throws {
        let library = try openRecord()
        let doc = try XCTUnwrap(library.document)
        let wave = Waveform.build(chunks: doc.chunks, durationMs: 30_000, buckets: 30)
        XCTAssertEqual(wave.you.count, 30)
        XCTAssertEqual(wave.you.firstIndex(where: { $0 > 0.5 }), 12)
        XCTAssertEqual(wave.them.firstIndex(where: { $0 > 0.5 }), 20, "placed by its chunk's start")
        XCTAssertEqual(wave.you[5], 0)
    }

    func testASliceCursorWalksChunksFromAMomentAndSkipsGaps() {
        let a = TimelineChunk(channel: .mic, path: "a", startMs: 0, frames: 16_000, sampleRate: 16_000, channels: 1, dataOffset: 64)
        let b = TimelineChunk(channel: .mic, path: "b", startMs: 5_000, frames: 32_000, sampleRate: 16_000, channels: 1, dataOffset: 64)
        var cursor = SliceCursor(chunks: [b, a], fromMs: 500)
        XCTAssertEqual(cursor.next(seconds: 4), ChunkSlice(chunk: a, firstFrame: 8_000, frameCount: 8_000))
        XCTAssertEqual(cursor.next(seconds: 1.5), ChunkSlice(chunk: b, firstFrame: 0, frameCount: 24_000))
        XCTAssertEqual(cursor.next(seconds: 4)?.startSeconds, 6.5)
        XCTAssertNil(cursor.next(seconds: 4))
        var inGap = SliceCursor(chunks: [a, b], fromMs: 2_000)
        XCTAssertEqual(inGap.next(seconds: 1)?.chunk.path, "b", "a moment in a gap starts at the next chunk")
    }

    func testAChunkReadsAsItsFramesDeinterleaved() throws {
        let url = directory.appendingPathComponent("far-000001-8000x2.pcm")
        try writeChunk(url, samples: [0.1, -0.1, 0.2, -0.2, 0.3, -0.3])
        let chunk = TimelineChunk(channel: .far, path: url.path, startMs: 0, frames: 3, sampleRate: 8_000, channels: 2, dataOffset: 64)
        let buffer = try ChunkAudio.buffer(ChunkSlice(chunk: chunk, firstFrame: 1, frameCount: 2))
        XCTAssertEqual(buffer.frameLength, 2)
        XCTAssertEqual(buffer.format.channelCount, 2)
        XCTAssertEqual(buffer.floatChannelData?[0][0], 0.2)
        XCTAssertEqual(buffer.floatChannelData?[1][1], -0.3)
        let missing = TimelineChunk(channel: .far, path: directory.appendingPathComponent("gone.pcm").path, startMs: 0, frames: 3, sampleRate: 8_000, channels: 2, dataOffset: 64)
        XCTAssertThrowsError(try ChunkAudio.buffer(ChunkSlice(chunk: missing, firstFrame: 0, frameCount: 1)))
    }

    // MARK: Review fixes

    /// Item 1: an estimated timeline (the meeting's start was not written) and chunks left out
    /// reach what the Record screen reads, with the words it shows; a precise one shows none.
    func testAnEstimatedTimelineAndMissingAudioReachTheScreensState() throws {
        let precise = try XCTUnwrap(try openRecord().document)
        XCTAssertEqual(precise.playbackCaveats.messages(waveformPartial: false), [])
        XCTAssertEqual(precise.ledgerStatus(blottedAt: nil), "Blotted · final")

        let library = try openRecord(timeline: "estimated", leftOut: 2)
        let doc = try XCTUnwrap(library.document)
        XCTAssertTrue(doc.playbackCaveats.timelineEstimated)
        XCTAssertEqual(doc.playbackCaveats.leftOut, 2)
        XCTAssertEqual(doc.playbackCaveats.messages(waveformPartial: false), [
            "Timing estimated: the two sides may be out of step.",
            "Part of this recording can't be played.",
        ])
        XCTAssertEqual(doc.ledgerStatus(blottedAt: nil), "Blotted · final · timing estimated")
        XCTAssertEqual(doc.playbackCaveats.messages(waveformPartial: true).last,
                       "Part of this recording can't be played.", "said once")
        XCTAssertEqual(RecordDocument.Caveats(timelineEstimated: false, leftOut: 0).messages(waveformPartial: true),
                       ["Part of this recording can't be played."])
    }

    /// Item 5: a chunk shorter on disk than its frame count is an error, never fewer samples with
    /// the cursor moving on by the full count (which would drift the rest of the side).
    func testATruncatedChunkIsAnErrorNotDrift() throws {
        let url = directory.appendingPathComponent("mic-000009-16000x1.pcm")
        try writeChunk(url, samples: [Float](repeating: 0.1, count: 50))
        let chunk = TimelineChunk(channel: .mic, path: url.path, startMs: 0, frames: 100, sampleRate: 16_000, channels: 1, dataOffset: 64)
        XCTAssertEqual(try ChunkAudio.samples(ChunkSlice(chunk: chunk, firstFrame: 0, frameCount: 50)).count, 50)
        XCTAssertThrowsError(try ChunkAudio.samples(ChunkSlice(chunk: chunk, firstFrame: 0, frameCount: 100))) { error in
            XCTAssertEqual(error as? ChunkAudio.ReadError, .short)
        }
        XCTAssertThrowsError(try ChunkAudio.samples(ChunkSlice(chunk: chunk, firstFrame: 60, frameCount: 10)))
    }

    /// Item 3: a chunk that cannot be read leaves its stretch flat, and the waveform says it is
    /// partial (the player bar shows that), rather than looking like silence.
    func testAWaveformWithAnUnreadableChunkIsMarkedPartial() throws {
        let doc = try XCTUnwrap(try openRecord().document)
        let whole = Waveform.build(chunks: doc.chunks, durationMs: 30_000, buckets: 30)
        XCTAssertFalse(whole.partial)
        var chunks = doc.chunks
        chunks.append(TimelineChunk(channel: .far, path: directory.appendingPathComponent("far-000001-16000x1.pcm").path,
                                    startMs: 28_000, frames: 16_000, sampleRate: 16_000, channels: 1, dataOffset: 64))
        let partial = Waveform.build(chunks: chunks, durationMs: 30_000, buckets: 30)
        XCTAssertTrue(partial.partial)
        XCTAssertEqual(partial.you.firstIndex(where: { $0 > 0.5 }), 12, "what could be read is still drawn")
    }

    /// Item 6: switching records quickly never shows the first record's waveform over the second's:
    /// the first build is cancelled, and a late result is checked against the record shown.
    func testARapidSwitchKeepsTheNewestRecordsWaveform() async throws {
        let long = try XCTUnwrap(try openRecord().document)
        let shortURL = directory.appendingPathComponent("mic-000000-16000x1-short.pcm")
        try writeChunk(shortURL, samples: samples(seconds: 2, tone: 0.5...1.0))
        let short = RecordDocument(try XCTUnwrap({
            guard case .libraryRecord(let r) = try InkEvent.decode(Data(#"""
            {"type":"library.record","ref":"x","record":{"record":"r2","kind":"meeting","started_at_unix_ms":0,"ended_at_unix_ms":2000,"revision":2,"has_audio":true},
             "segments":[],"notes":[],"commitments":[],"speakers":[],
             "audio":{"timeline":"recorded","left_out":0,"chunks":[{"channel":"mic","path":"\#(shortURL.path)","start_ms":0,"frames":32000,"sample_rate":16000,"channels":1,"data_offset":64}]}}
            """#.utf8)) else { return nil }
            return r
        }()))
        let loader = WaveformLoader()
        loader.load(long, buckets: 40)
        loader.load(short, buckets: 40)
        await loader.settled()
        XCTAssertEqual(loader.record, "r2")
        XCTAssertEqual(loader.waveform, Waveform.build(chunks: short.chunks, durationMs: short.durationMs, buckets: 40))
        // And the other way round.
        loader.load(short, buckets: 40)
        loader.load(long, buckets: 40)
        await loader.settled()
        XCTAssertEqual(loader.record, "r1")
        XCTAssertEqual(loader.waveform.you.firstIndex(where: { $0 > 0.5 }), Waveform.build(chunks: long.chunks, durationMs: long.durationMs, buckets: 40).you.firstIndex(where: { $0 > 0.5 }))
    }

    /// Item 7: the output changing under a playing engine (a device unplugged, the route moved)
    /// restarts playback in place, from where it was, rather than going silent unnoticed.
    func testAnOutputChangeRestartsPlaybackInPlace() async throws {
        let library = try openRecord()
        let player = try XCTUnwrap(library.player)
        library.playFrom(12_400)
        _ = try await listen(player)
        let before = player.positionMs()
        let engine = try XCTUnwrap(player.engineForTests)
        NotificationCenter.default.post(name: .AVAudioEngineConfigurationChange, object: engine)
        XCTAssertEqual(player.state, .playing)
        XCTAssertFalse(player.engineForTests === engine, "a new engine")
        let heard = try await listen(player)
        XCTAssertLessThanOrEqual(abs(player.positionMs() - before), 1_000, "from where it was")
        XCTAssertGreaterThan(rms(heard), 0.05, "and it plays")

        // Paused, a change only lets the engine go; Play starts a new one.
        player.pause()
        let paused = try XCTUnwrap(player.engineForTests)
        NotificationCenter.default.post(name: .AVAudioEngineConfigurationChange, object: paused)
        XCTAssertEqual(player.state, .paused)
        XCTAssertNil(player.engineForTests)
        player.stop()
    }

    /// Re-check fix: AVAudioEngine stops itself before it posts the configuration change, and a
    /// stopped engine's nodes have no render time. The player keeps its own clock of the run (host
    /// time on a device; frames rendered offline), so it resumes from where the listener was, not
    /// from the last play or seek.
    func testAnOutputChangeAfterTheEngineStoppedResumesWhereItWasNotAtTheAnchor() async throws {
        let library = try openRecord()
        let player = try XCTUnwrap(library.player)
        library.playFrom(5_000)
        for _ in 0..<4 { _ = try await player.renderOffline(frames: 48_000) }  // 4 s
        let before = player.positionMs()
        XCTAssertGreaterThan(before, 8_500, "well past the anchor")
        let engine = try XCTUnwrap(player.engineForTests)
        engine.stop()  // as AVAudioEngine does before posting
        XCTAssertEqual(player.positionMs(), before, "a stopped engine does not lose the position")
        NotificationCenter.default.post(name: .AVAudioEngineConfigurationChange, object: engine)
        XCTAssertEqual(player.state, .playing)
        XCTAssertLessThanOrEqual(abs(player.anchorMs - before), 500, "resumed at \(player.anchorMs), was at \(before)")
        player.stop()
    }

    /// Recorded-call check: right after the final pass the record's player showed Pause. Playback
    /// starts only from the user: opening a record, the final pass finishing (which opens the shown
    /// record again) and its later edits never start it, nor start again a record the user paused.
    /// Every record.open is answered as the core would.
    func testOpeningARecordAndTheFinalPassNeverStartPlayback() async throws {
        let core = AnsweringCore(answer: { self.answer(request: $0) })
        let library = LibraryModel(send: { core.sent.append($0.json) })
        core.library = library
        library.makePlayer = { RecordPlayer(document: $0, output: .offline(sampleRate: 48_000, channels: 2)) }
        func event(_ json: String) throws -> InkEvent { try InkEvent.decode(Data(json.utf8)) }
        func assertNotPlaying(_ when: String) throws {
            let player = try XCTUnwrap(library.player, when)
            XCTAssertNotEqual(player.state, .playing, when)
            XCTAssertNil(player.engineForTests, "\(when): no engine, so nothing can sound")
        }

        library.open("r1")  // a click in the list, or Today's Open record
        try core.answerOpens()
        try assertNotPlaying("opened")
        library.apply([try event(#"{"type":"meeting.finished","record":"r1","revision":2}"#)])
        XCTAssertEqual(try core.answerOpens(), 1, "the final pass opens the shown record again")
        try assertNotPlaying("the final pass finished")
        library.apply([try event(#"{"type":"commitment.updated","commitment":"c1","done":true}"#)])
        try core.answerOpens()
        try assertNotPlaying("the record changed")

        // Played by the user (Today's Play), then paused: the next refresh leaves it paused.
        library.open("r1", seekMs: 12_400, play: true)
        try core.answerOpens()
        let player = try XCTUnwrap(library.player)
        XCTAssertEqual(player.state, .playing, "the user's Play plays")
        _ = try await listen(player)
        player.pause()
        library.apply([try event(#"{"type":"meeting.finished","record":"r1","revision":3}"#)])
        try core.answerOpens()
        XCTAssertEqual(player.state, .paused, "a refresh never presses Play again")
        player.stop()
    }

    /// Item 7: when playback cannot start again after an output change, it says so (.failed with
    /// its words), never plays nothing as if all were well.
    func testAnOutputChangeThatCannotRestartFailsVisibly() async throws {
        let library = try openRecord()
        let player = try XCTUnwrap(library.player)
        library.playFrom(12_400)
        _ = try await listen(player)
        let engine = try XCTUnwrap(player.engineForTests)
        // The files go before the change: the new engine cannot read them.
        try FileManager.default.removeItem(at: directory)
        NotificationCenter.default.post(name: .AVAudioEngineConfigurationChange, object: engine)
        XCTAssertEqual(player.state, .failed("This recording can't be played right now."))
    }
}

/// The core's side of record.open for a test: each one sent is answered with the record.
@MainActor
private final class AnsweringCore {
    var sent: [String] = []
    weak var library: LibraryModel?
    private var answered = 0
    private let answer: (String) -> String

    init(answer: @escaping (String) -> String) {
        self.answer = answer
    }

    /// Answers every record.open sent since the last call; returns how many there were.
    @discardableResult
    func answerOpens() throws -> Int {
        var opens = 0
        while answered < sent.count {
            let command = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(sent[answered].utf8)) as? [String: Any])
            answered += 1
            guard command["cmd"] as? String == "record.open", let id = command["id"] as? String else { continue }
            opens += 1
            library?.apply([try InkEvent.decode(Data(answer(id).utf8))])
        }
        return opens
    }
}
