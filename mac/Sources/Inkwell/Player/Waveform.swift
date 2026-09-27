// The player's two-lane waveform: them above, you below, one peak per bucket of the record's
// timeline. Built off the main actor by reading the chunks a second at a time: an hour of audio
// is never in memory, only its few hundred peaks.
import Foundation
import InkBridge
import Observation
import os

struct Waveform: Equatable, Sendable {
    /// Your side's peaks, 0 to 1, one per bucket.
    let you: [Float]
    /// Their side's peaks.
    let them: [Float]
    /// A chunk could not be read: its stretch is flat, which is not silence. The player bar says
    /// so.
    var partial = false

    static let empty = Waveform(you: [], them: [])

    private static let log = Logger(subsystem: "com.inkwell.app", category: "player")

    /// **Background.** Reads `chunks` and peaks them into `buckets` over `durationMs`. Each lane is
    /// scaled to its own loudest bucket (the far end is often quieter), with a square root so
    /// quiet speech still shows. A chunk that cannot be read leaves its buckets flat, is logged
    /// (its side and file name, never samples) and marks the waveform partial.
    static func build(chunks: [TimelineChunk], durationMs: Int64, buckets: Int) -> Waveform {
        guard buckets > 0, durationMs > 0 else { return .empty }
        var lanes: [Channel: [Float]] = [.mic: Array(repeating: 0, count: buckets), .far: Array(repeating: 0, count: buckets)]
        var partial = false
        let msPerBucket = Double(durationMs) / Double(buckets)
        for chunk in chunks where chunk.frames > 0 && chunk.sampleRate > 0 {
            var cursor = SliceCursor(chunks: [chunk], fromMs: chunk.startMs)
            while let slice = cursor.next(seconds: 1) {
                if Task.isCancelled { return .empty }
                let samples: [Float]
                do {
                    samples = try ChunkAudio.samples(slice)
                } catch {
                    let file = URL(fileURLWithPath: chunk.path).lastPathComponent
                    log.error("waveform: \(chunk.channel.rawValue, privacy: .public) chunk \(file, privacy: .public) could not be read (\(String(describing: error), privacy: .public)); drawn flat")
                    partial = true
                    break
                }
                let channels = max(chunk.channels, 1)
                let frames = samples.count / channels
                let rate = Double(chunk.sampleRate)
                var lane = lanes[chunk.channel] ?? []
                for f in 0..<frames {
                    var peak: Float = 0
                    for c in 0..<channels {
                        peak = max(peak, abs(samples[f * channels + c]))
                    }
                    let ms = slice.startSeconds * 1000 + Double(f) * 1000 / rate
                    let bucket = Int(ms / msPerBucket)
                    if bucket >= 0, bucket < buckets, peak > lane[bucket] {
                        lane[bucket] = peak
                    }
                }
                lanes[chunk.channel] = lane
            }
        }
        func scaled(_ lane: [Float]) -> [Float] {
            let top = lane.max() ?? 0
            guard top > 0 else { return lane }
            return lane.map { ($0 / top).squareRoot() }
        }
        return Waveform(you: scaled(lanes[.mic] ?? []), them: scaled(lanes[.far] ?? []), partial: partial)
    }
}

/// Builds the open record's waveform off the main actor, one record at a time: a new record
/// cancels the build under way, and a result that arrives for a record no longer shown is dropped,
/// so a quick switch never shows the previous record's waveform.
@MainActor
@Observable
final class WaveformLoader {
    private(set) var waveform = Waveform.empty
    /// The record `waveform` is (or is being built) for.
    private(set) var record: String?

    @ObservationIgnored private var build: Task<Waveform, Never>?
    @ObservationIgnored private var delivery: Task<Void, Never>?

    /// Starts building `document`'s waveform, replacing whatever was being built.
    func load(_ document: RecordDocument, buckets: Int) {
        cancel()
        let id = document.record.record
        record = id
        waveform = .empty
        let chunks = document.chunks
        let duration = document.durationMs
        // Detached: a second of audio at a time, off the main actor. Its handle is kept, so a
        // newer record cancels it.
        let build = Task.detached(priority: .utility) {
            Waveform.build(chunks: chunks, durationMs: duration, buckets: buckets)
        }
        self.build = build
        delivery = Task { [weak self] in
            let built = await build.value
            guard let self, !build.isCancelled, self.record == id else { return }
            self.waveform = built
        }
    }

    /// Stops the build under way (the record closed, or another opened).
    func cancel() {
        build?.cancel()
        build = nil
    }

    /// Waits until the latest build has been delivered or dropped (tests).
    func settled() async {
        await delivery?.value
    }
}
