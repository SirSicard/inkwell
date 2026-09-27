// Plays a record: AVAudioEngine with one player node per side (You, the mic; Them, the far end),
// both on the engine's clock and started at the same moment, each with its own volume (the
// You/Them mix). Audio stays on disk: each side keeps two slices of a few seconds queued, and
// schedules the next as one finishes. Every slice is placed by its own time on the record's
// timeline, so the sides stay aligned across gaps, and a line's stamp and the audio agree.
//
// The engine exists only while something plays or is paused: an idle Record screen holds no audio
// engine and draws nothing (architecture rule 9).
//
// When the output changes under it (a device unplugged, the route moved: the engine's
// configuration-change notice), the engine is let go and playback starts again in place, from
// where it was; if it cannot, the player fails with its words rather than going silent. Tests render offline (manual rendering), which
// needs no audio device.
import AVFoundation
import Foundation
import InkBridge
import Observation
import os

@MainActor
@Observable
final class RecordPlayer {
    enum State: Equatable, Sendable {
        case idle
        case playing
        case paused
        case ended
        /// The audio could not play (no output device, unreadable files). Said on screen.
        case failed(String)
    }

    /// Where the engine's output goes.
    enum Output: Sendable {
        /// The Mac's output device.
        case device
        /// Offline, in this format: the test renders the frames itself.
        case offline(sampleRate: Double, channels: AVAudioChannelCount)
    }

    private(set) var state: State = .idle
    /// Where the current run started (or where a paused one stopped), ms on the record's timeline.
    private(set) var anchorMs: Int64 = 0
    /// Your side's volume, 0 to 1.
    var youVolume: Float = 1 {
        didSet { nodes[.mic]?.volume = youVolume }
    }
    /// Their side's volume, 0 to 1.
    var themVolume: Float = 1 {
        didSet { nodes[.far]?.volume = themVolume }
    }

    let durationMs: Int64
    /// The sides this record has audio for.
    let sides: [Channel]

    var isPlaying: Bool { state == .playing }

    /// How much of each side is queued ahead: two slices of this many seconds.
    static let sliceSeconds = 4.0
    static let slicesAhead = 2

    @ObservationIgnored private let chunks: [Channel: [TimelineChunk]]
    @ObservationIgnored private let output: Output
    @ObservationIgnored private var engine: AVAudioEngine?
    /// The engine's configuration-change observer. Main actor; removed with the engine.
    @ObservationIgnored private var configurationObserver: NSObjectProtocol?
    @ObservationIgnored private var nodes: [Channel: AVAudioPlayerNode] = [:]
    @ObservationIgnored private var formats: [Channel: AVAudioFormat] = [:]
    @ObservationIgnored private var cursors: [Channel: SliceCursor] = [:]
    @ObservationIgnored private var queued: [Channel: Int] = [:]
    @ObservationIgnored private var exhausted: Set<Channel> = []
    /// Raised by every seek, pause and stop: a completion from an earlier run changes nothing.
    @ObservationIgnored private var run = 0
    @ObservationIgnored private let log = Logger(subsystem: "com.inkwell.app", category: "player")

    init(document: RecordDocument, output: Output = .device) {
        var bySide: [Channel: [TimelineChunk]] = [:]
        for chunk in document.chunks {
            bySide[chunk.channel, default: []].append(chunk)
        }
        chunks = bySide
        sides = [Channel.mic, .far].filter { bySide[$0] != nil }
        durationMs = document.durationMs
        self.output = output
    }

    // MARK: Control

    /// Plays from the playhead.
    func play() {
        guard !sides.isEmpty, state != .playing else { return }
        if state == .ended {
            anchorMs = 0
        }
        do {
            try startEngine()
            try startRun()
            state = .playing
        } catch {
            fail(error)
        }
    }

    /// Stops where it is; play goes on from there.
    func pause() {
        guard state == .playing else { return }
        anchorMs = positionMs()
        stopNodes()
        engine?.pause()
        state = .paused
    }

    /// Plays or pauses.
    func toggle() {
        isPlaying ? pause() : play()
    }

    /// Moves the playhead to `ms`, playing on from there if it was playing.
    func seek(toMs ms: Int64) {
        let target = min(max(ms, 0), durationMs)
        let wasPlaying = state == .playing
        stopNodes()
        anchorMs = target
        if wasPlaying {
            do {
                try startRun()
            } catch {
                fail(error)
            }
        } else if state == .ended {
            state = .paused
        }
    }

    /// Lets go of the engine.
    func stop() {
        releaseEngine()
        if state == .playing || state == .paused || state == .ended {
            state = .idle
        }
    }

    /// Stops and drops the engine and its nodes, and stops listening to it.
    private func releaseEngine() {
        stopNodes()
        if let configurationObserver {
            NotificationCenter.default.removeObserver(configurationObserver)
        }
        configurationObserver = nil
        engine?.stop()
        engine = nil
        nodes = [:]
        formats = [:]
    }

    /// The engine's output changed (AVAudioEngineConfigurationChange): the engine has stopped. A
    /// new one starts from where playback was; paused, the next Play starts it.
    private func configurationChanged() {
        let wasPlaying = state == .playing
        let at = positionMs()
        releaseEngine()
        anchorMs = at
        guard wasPlaying else { return }
        do {
            try startEngine()
            try startRun()
            state = .playing
        } catch {
            fail(error)
        }
    }

    /// The engine now (tests: to post its configuration change).
    var engineForTests: AVAudioEngine? { engine }

    /// Where the playhead is now, ms on the record's timeline. Read while drawing (the view
    /// redraws only while playing); not observed.
    func positionMs() -> Int64 {
        guard state == .playing,
            let node = sides.lazy.compactMap({ self.nodes[$0] }).first,
            let nodeTime = node.lastRenderTime,
            let playerTime = node.playerTime(forNodeTime: nodeTime)
        else { return anchorMs }
        let played = Double(max(playerTime.sampleTime, 0)) / playerTime.sampleRate
        return min(anchorMs + Int64(played * 1000), durationMs)
    }

    // MARK: The engine

    private func startEngine() throws {
        if let engine {
            if !engine.isRunning { try engine.start() }
            return
        }
        let engine = AVAudioEngine()
        if case .offline(let rate, let channels) = output {
            guard let format = AVAudioFormat(standardFormatWithSampleRate: rate, channels: channels) else {
                throw PlayerError.format
            }
            try engine.enableManualRenderingMode(.offline, format: format, maximumFrameCount: 4096)
        }
        for side in sides {
            guard let first = chunks[side]?.first, let format = ChunkAudio.format(of: first) else { continue }
            let node = AVAudioPlayerNode()
            engine.attach(node)
            engine.connect(node, to: engine.mainMixerNode, format: format)
            node.volume = side == .mic ? youVolume : themVolume
            nodes[side] = node
            formats[side] = format
        }
        engine.prepare()
        try engine.start()
        self.engine = engine
        configurationObserver = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.configurationChanged() }
        }
    }

    /// Queues each side from `anchorMs` and starts both nodes at one moment.
    private func startRun() throws {
        run += 1
        exhausted = []
        for side in sides {
            cursors[side] = SliceCursor(chunks: chunks[side] ?? [], fromMs: anchorMs)
            queued[side] = 0
            try fill(side)
        }
        if case .device = output {
            // One host time for both, a moment ahead: sample-aligned whatever the render cycle.
            let at = AVAudioTime(hostTime: mach_absolute_time() + AVAudioTime.hostTime(forSeconds: 0.05))
            for side in sides { nodes[side]?.play(at: at) }
        } else {
            // Offline, nothing renders between these calls: both start on the next frame.
            for side in sides { nodes[side]?.play() }
        }
    }

    /// Keeps `slicesAhead` slices of `side` queued.
    private func fill(_ side: Channel) throws {
        guard let node = nodes[side], let format = formats[side] else { return }
        while (queued[side] ?? 0) < Self.slicesAhead {
            guard let slice = cursors[side]?.next(seconds: Self.sliceSeconds) else {
                exhausted.insert(side)
                break
            }
            let read = try ChunkAudio.buffer(slice)
            guard let buffer = ChunkAudio.convert(read, to: format) else { throw PlayerError.format }
            // Its place on the node's timeline, which starts at the anchor.
            let seconds = max(slice.startSeconds - Double(anchorMs) / 1000, 0)
            let at = AVAudioTime(sampleTime: AVAudioFramePosition((seconds * format.sampleRate).rounded()), atRate: format.sampleRate)
            let run = self.run
            queued[side, default: 0] += 1
            // Rendered, not played back: the next slice is queued as soon as this one has gone to
            // the output, and offline rendering (the tests) reports it too.
            node.scheduleBuffer(buffer, at: at, options: [], completionCallbackType: .dataRendered) { [weak self] _ in
                Task { @MainActor in self?.played(side, run: run) }
            }
        }
    }

    private func played(_ side: Channel, run: Int) {
        guard run == self.run, state == .playing else { return }
        queued[side, default: 1] -= 1
        do {
            try fill(side)
        } catch {
            fail(error)
            return
        }
        if sides.allSatisfy({ exhausted.contains($0) && (queued[$0] ?? 0) == 0 }) {
            stopNodes()
            engine?.pause()
            anchorMs = durationMs
            state = .ended
        }
    }

    private func stopNodes() {
        // Stopping a node calls back for every queued buffer; the new run number ignores them.
        run += 1
        for node in nodes.values { node.stop() }
        queued = [:]
    }

    private func fail(_ error: Error) {
        // The error names a file or a status, never what was said.
        log.error("playback failed: \(String(describing: error), privacy: .public)")
        stop()
        state = .failed("This recording can't be played right now.")
    }

    enum PlayerError: Error {
        case format
    }

    // MARK: Offline rendering (tests)

    /// Renders `frames` of output offline and returns them. Only with `.offline` output, after
    /// `play`.
    func renderOffline(frames: AVAudioFrameCount) async throws -> AVAudioPCMBuffer? {
        guard case .offline = output, let engine,
            let buffer = AVAudioPCMBuffer(pcmFormat: engine.manualRenderingFormat, frameCapacity: frames)
        else { return nil }
        var rendered: AVAudioFrameCount = 0
        guard let whole = AVAudioPCMBuffer(pcmFormat: engine.manualRenderingFormat, frameCapacity: frames) else { return nil }
        // Until the frames are rendered, or playback ends (which pauses the engine).
        while rendered < frames, engine.isRunning {
            let step = min(frames - rendered, engine.manualRenderingMaximumFrameCount)
            buffer.frameLength = 0
            let status = try engine.renderOffline(step, to: buffer)
            guard status == .success else { break }
            let n = Int(buffer.frameLength)
            if let src = buffer.floatChannelData, let dst = whole.floatChannelData {
                for c in 0..<Int(buffer.format.channelCount) {
                    (dst[c] + Int(rendered)).update(from: src[c], count: n)
                }
            }
            rendered += buffer.frameLength
            whole.frameLength = rendered
            // Completions hop to the main actor: let them run so the next slices get queued.
            await Task.yield()
        }
        return whole
    }
}
