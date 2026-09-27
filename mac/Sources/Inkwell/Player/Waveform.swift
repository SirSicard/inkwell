// The player's two-lane waveform: them above, you below, one peak per bucket of the record's
// timeline. Built off the main actor by reading the chunks a second at a time: an hour of audio
// is never in memory, only its few hundred peaks.
import Foundation
import InkBridge

struct Waveform: Equatable, Sendable {
    /// Your side's peaks, 0 to 1, one per bucket.
    let you: [Float]
    /// Their side's peaks.
    let them: [Float]

    static let empty = Waveform(you: [], them: [])

    /// **Background.** Reads `chunks` and peaks them into `buckets` over `durationMs`. Each lane is
    /// scaled to its own loudest bucket (the far end is often quieter), with a square root so
    /// quiet speech still shows. A chunk that cannot be read leaves its buckets flat.
    static func build(chunks: [TimelineChunk], durationMs: Int64, buckets: Int) -> Waveform {
        guard buckets > 0, durationMs > 0 else { return .empty }
        var lanes: [Channel: [Float]] = [.mic: Array(repeating: 0, count: buckets), .far: Array(repeating: 0, count: buckets)]
        let msPerBucket = Double(durationMs) / Double(buckets)
        for chunk in chunks where chunk.frames > 0 && chunk.sampleRate > 0 {
            var cursor = SliceCursor(chunks: [chunk], fromMs: chunk.startMs)
            while let slice = cursor.next(seconds: 1) {
                if Task.isCancelled { return .empty }
                guard let samples = try? ChunkAudio.samples(slice) else { break }
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
        return Waveform(you: scaled(lanes[.mic] ?? []), them: scaled(lanes[.far] ?? []))
    }
}
