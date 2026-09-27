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
    private func answer(request: String) -> String {
        let mic = directory.appendingPathComponent("mic-000000-16000x1.pcm").path
        let far = directory.appendingPathComponent("far-000000-16000x1.pcm").path
        return #"""
        {"type":"library.record","ref":"\#(request)",
         "record":{"record":"r1","kind":"meeting","title":"Tones","started_at_unix_ms":0,"ended_at_unix_ms":30000,"revision":2,"has_audio":true},
         "segments":[{"channel":"mic","start_ms":12000,"end_ms":13000,"text":"a tone from you"},
                     {"channel":"far","start_ms":20100,"end_ms":21000,"text":"a tone from them"}],
         "notes":[{"note":"n1","at_ms":12400,"text":"you spoke here"}],
         "commitments":[],"speakers":[],
         "audio":{"timeline":"recorded","chunks":[
           {"channel":"mic","path":"\#(mic)","start_ms":0,"frames":480000,"sample_rate":16000,"channels":1,"data_offset":64},
           {"channel":"far","path":"\#(far)","start_ms":2000,"frames":448000,"sample_rate":16000,"channels":1,"data_offset":64}]}}
        """#
    }

    /// A library with the record open, its player rendering offline.
    private func openRecord() throws -> LibraryModel {
        var sent: [String] = []
        let library = LibraryModel(send: { sent.append($0.json) })
        library.makePlayer = { RecordPlayer(document: $0, output: .offline(sampleRate: 48_000, channels: 2)) }
        library.open("r1")
        let id = ((try JSONSerialization.jsonObject(with: Data(sent.last!.utf8))) as? [String: Any])?["id"] as? String ?? ""
        library.apply([try InkEvent.decode(Data(answer(request: id).utf8))])
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
}
