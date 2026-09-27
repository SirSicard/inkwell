// A record's audio as the player reads it: the core's chunk files (raw little-endian float
// samples, interleaved, after a header the core names the size of), read a few seconds at a time.
// Disk is where the audio lives; the player never holds more than the slices it has queued.
import AVFoundation
import Foundation
import InkBridge

/// A stretch of one chunk.
struct ChunkSlice: Equatable, Sendable {
    let chunk: TimelineChunk
    let firstFrame: Int64
    let frameCount: Int64

    /// Where it starts on the record's timeline, in seconds.
    var startSeconds: Double {
        Double(chunk.startMs) / 1000 + Double(firstFrame) / Double(max(chunk.sampleRate, 1))
    }
}

/// Walks one side's chunks from a moment on, a slice at a time. Gaps between chunks (a side that
/// started late, or a stretch lost) are skipped: each slice carries its own place on the timeline,
/// and silence fills the rest.
struct SliceCursor: Equatable, Sendable {
    private let chunks: [TimelineChunk]
    private var index = 0
    private var frame: Int64 = 0

    /// `chunks` of one side, from `ms` on the record's timeline.
    init(chunks: [TimelineChunk], fromMs ms: Int64) {
        self.chunks = chunks.filter { $0.frames > 0 && $0.sampleRate > 0 }.sorted { $0.startMs < $1.startMs }
        index = self.chunks.firstIndex(where: { $0.endMs > ms }) ?? self.chunks.count
        if index < self.chunks.count {
            let chunk = self.chunks[index]
            frame = max(0, (ms - chunk.startMs) * Int64(chunk.sampleRate) / 1000)
        }
    }

    /// The next slice, at most `seconds` long; nil at the end.
    mutating func next(seconds: Double) -> ChunkSlice? {
        while index < chunks.count {
            let chunk = chunks[index]
            if frame >= chunk.frames {
                index += 1
                frame = 0
                continue
            }
            let count = min(Int64(seconds * Double(chunk.sampleRate)), chunk.frames - frame)
            let slice = ChunkSlice(chunk: chunk, firstFrame: frame, frameCount: max(count, 1))
            frame += slice.frameCount
            return slice
        }
        return nil
    }
}

enum ChunkAudio {
    enum ReadError: Error, Equatable {
        case unreadable(String)
        case short
    }

    /// The standard (deinterleaved float) format of `chunk`.
    static func format(of chunk: TimelineChunk) -> AVAudioFormat? {
        AVAudioFormat(
            standardFormatWithSampleRate: Double(chunk.sampleRate), channels: AVAudioChannelCount(max(chunk.channels, 1)))
    }

    /// The interleaved samples of `slice`, as stored.
    static func samples(_ slice: ChunkSlice) throws -> [Float] {
        let chunk = slice.chunk
        guard let handle = FileHandle(forReadingAtPath: chunk.path) else {
            throw ReadError.unreadable(URL(fileURLWithPath: chunk.path).lastPathComponent)
        }
        defer { try? handle.close() }
        let bytesPerFrame = UInt64(max(chunk.channels, 1)) * 4
        try handle.seek(toOffset: UInt64(chunk.dataOffset) + UInt64(slice.firstFrame) * bytesPerFrame)
        let wanted = Int(UInt64(slice.frameCount) * bytesPerFrame)
        guard let data = try handle.read(upToCount: wanted), !data.isEmpty else { throw ReadError.short }
        let count = data.count / 4
        var samples = [Float](repeating: 0, count: count)
        data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            for i in 0..<count {
                let bits = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: i * 4, as: UInt32.self))
                samples[i] = Float(bitPattern: bits)
            }
        }
        return samples
    }

    /// `slice` as a buffer in its chunk's format, deinterleaved.
    static func buffer(_ slice: ChunkSlice) throws -> AVAudioPCMBuffer {
        let chunk = slice.chunk
        let samples = try samples(slice)
        let channels = max(chunk.channels, 1)
        let frames = samples.count / channels
        guard let format = format(of: chunk),
            let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(max(frames, 1))),
            let out = buffer.floatChannelData
        else { throw ReadError.unreadable("format") }
        for c in 0..<channels {
            let lane = out[c]
            for f in 0..<frames {
                lane[f] = samples[f * channels + c]
            }
        }
        buffer.frameLength = AVAudioFrameCount(frames)
        return buffer
    }

    /// `buffer` in `format` (a side whose device changed rate mid-meeting). Nil if it cannot be.
    static func convert(_ buffer: AVAudioPCMBuffer, to format: AVAudioFormat) -> AVAudioPCMBuffer? {
        if buffer.format == format { return buffer }
        guard let converter = AVAudioConverter(from: buffer.format, to: format) else { return nil }
        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 32
        guard let out = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { return nil }
        let given = GivenOnce(buffer)
        var error: NSError?
        let status = converter.convert(to: out, error: &error) { _, inputStatus in
            if let next = given.take() {
                inputStatus.pointee = .haveData
                return next
            }
            inputStatus.pointee = .endOfStream
            return nil
        }
        return status == .error ? nil : out
    }
}

/// Hands a buffer to a converter's input block once. The block runs synchronously inside
/// `convert`, on the calling thread, so the unsynchronised state is never shared: @unchecked is
/// for the block's @Sendable signature only.
private final class GivenOnce: @unchecked Sendable {
    private var buffer: AVAudioPCMBuffer?

    init(_ buffer: AVAudioPCMBuffer) {
        self.buffer = buffer
    }

    func take() -> AVAudioPCMBuffer? {
        defer { buffer = nil }
        return buffer
    }
}
