// Parakeet with the real models on the three AMI IHM clips the engine choice was measured on
// (709 reference words). Skipped unless INK_BENCH_DIR and INK_MODELS_DIR are set: CI has no
// models. Run locally, one timing run at a time on an otherwise quiet Mac:
//
//   INK_BENCH_DIR=<bench data> INK_MODELS_DIR=<models directory> \
//     swift test --package-path mac --filter AmiHarnessTests
//
// Reads $INK_BENCH_DIR/ami-ihm.tsv and ami-ihm/*.wav (column 2 names the WAV, column 4 holds the
// reference, as the core's Rust bench reader takes them); the models come from $INK_MODELS_DIR, a
// core's models directory with the parakeet-tdt-0.6b-v3-coreml row installed (the app's, once it
// has downloaded Parakeet), and are never downloaded. What it measured goes to
// $INK_BENCH_DIR/out/apple-engines/, never into the repository.
//
// 1. The model through the engine's decoder, clip by clip, as the choice was measured: the
//    corpus WER must be within 0.3 of the engine's registered rate.
// 2. The live stream through the core: each clip replayed as a meeting's mic, in real time, into
//    the registered streaming engine: the live finals' WER, partial latency and flicker (defined
//    in `LiveMeasure`). The same replay's final pass goes through Parakeet registered as an offline
//    engine, over the regions the core cuts. Neither is held to the measured rate: the trailing
//    window decodes each utterance on its own, and the final pass decodes the core's regions
//    after its gain stage, where the measurement decoded each clip whole. Both are reported; the
//    live finals are guarded against falling more than 3 points behind it. The final pass is
//    reported only: a real-time replay hands it slightly different audio each run (22 to 29 % over
//    five runs), so one run's number says little.
@testable import AppleEngines
import AVFoundation
import Foundation
import InkBridge
import Synchronization
import XCTest

private struct Clip {
    var wav: URL
    var reference: String
}

private func bench() throws -> URL {
    guard let dir = ProcessInfo.processInfo.environment["INK_BENCH_DIR"], !dir.isEmpty else {
        throw XCTSkip("set INK_BENCH_DIR to run the real-model harness")
    }
    guard let models = ProcessInfo.processInfo.environment["INK_MODELS_DIR"], !models.isEmpty else {
        throw XCTSkip("set INK_MODELS_DIR to a models directory with parakeet-tdt-0.6b-v3-coreml installed")
    }
    ParakeetModel.useModelsDirectory(URL(fileURLWithPath: models, isDirectory: true))
    return URL(fileURLWithPath: dir)
}

private func clips(_ bench: URL) throws -> [Clip] {
    let tsv = try String(contentsOf: bench.appendingPathComponent("ami-ihm.tsv"), encoding: .utf8)
    return try tsv.split(separator: "\n").map { line in
        let fields = line.split(separator: "\t", omittingEmptySubsequences: false).map(String.init)
        guard fields.count > 3, !fields[1].isEmpty else {
            throw XCTSkip("ami-ihm.tsv: a row without a WAV and a reference")
        }
        return Clip(wav: bench.appendingPathComponent("ami-ihm").appendingPathComponent(fields[1]), reference: fields[3])
    }
}

/// A 16 kHz mono float WAV.
private func read(_ url: URL) throws -> [Float] {
    let file = try AVAudioFile(forReading: url)
    XCTAssertEqual(file.processingFormat.sampleRate, 16_000, url.lastPathComponent)
    XCTAssertEqual(file.processingFormat.channelCount, 1, url.lastPathComponent)
    let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)))
    try file.read(into: buffer)
    let data = try XCTUnwrap(buffer.floatChannelData)
    return Array(UnsafeBufferPointer(start: data[0], count: Int(buffer.frameLength)))
}

private func output(_ bench: URL) throws -> URL {
    let out = bench.appendingPathComponent("out/apple-engines")
    try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
    return out
}

/// Nearest-rank percentile of a sorted array.
private func percentile(_ sorted: [Double], _ p: Double) -> Double {
    guard !sorted.isEmpty else { return .nan }
    let rank = Int((p / 100 * Double(sorted.count)).rounded(.up)) - 1
    return sorted[min(max(rank, 0), sorted.count - 1)]
}

private func stats(_ values: [Double]) -> [String: Double] {
    let s = values.sorted()
    return ["n": Double(s.count), "p50": percentile(s, 50), "p90": percentile(s, 90), "p95": percentile(s, 95),
            "max": s.last ?? .nan, "mean": s.isEmpty ? .nan : s.reduce(0, +) / Double(s.count)]
}

private func ms(_ d: Duration) -> Double {
    Double(d.components.seconds) * 1e3 + Double(d.components.attoseconds) / 1e15
}

/// The engine's words, one token each, compared without case or punctuation.
private func tokens(_ text: String) -> [String] {
    text.split(separator: " ").map { Wer.normalise(String($0)).joined() }
}

/// The offline engine, recording what it answered, in order.
private final class RecordingOffline: InkOfflineEngine {
    let inner = ParakeetOfflineEngine()
    private let answers = Mutex<[String]>([])
    private let inputs = Mutex<[String]>([])
    /// Each call's input: its length and a checksum, to tell whether the core handed the same audio
    /// from one run to the next.
    var received: [String] { inputs.withLock { $0 } }
    var id: String { inner.id }
    var licence: String { inner.licence }
    var jobs: [(job: Job, wer: Double)] { inner.jobs }
    var all: [String] { answers.withLock { $0 } }

    func transcribe(
        _ samples: [Float], channel: Channel, context: String?,
        completion: @escaping @Sendable (Result<[InkSegment], InkEngineError>) -> Void
    ) {
        var sum: UInt64 = 14_695_981_039_346_656_037
        for sample in samples {
            sum = (sum ^ UInt64(sample.bitPattern)) &* 1_099_511_628_211
        }
        inputs.withLock { $0.append("\(samples.count):\(String(sum, radix: 16))") }
        inner.transcribe(samples, channel: channel, context: context) { result in
            if case .success(let segments) = result {
                self.answers.withLock { $0.append(segments.map(\.text).joined(separator: " ")) }
            }
            completion(result)
        }
    }
}

/// Words of `old` that `new` drops or changes, in a minimal word alignment (substitutions and
/// deletions; a word inserted before them does not count them as changed).
func changedWords(_ old: [String], _ new: [String]) -> Int {
    // Each cell: (edits, substitutions + deletions), minimising edits, then changed words.
    var previous = (0...new.count).map { ($0, 0) }
    for (i, word) in old.enumerated() {
        var current = [(i + 1, i + 1)] + Array(repeating: (0, 0), count: new.count)
        for (j, other) in new.enumerated() {
            let same = word == other
            let diagonal = (previous[j].0 + (same ? 0 : 1), previous[j].1 + (same ? 0 : 1))
            let deletion = (previous[j + 1].0 + 1, previous[j + 1].1 + 1)
            let insertion = (current[j].0 + 1, current[j].1)
            current[j + 1] = [diagonal, deletion, insertion].min { $0.0 != $1.0 ? $0.0 < $1.0 : $0.1 < $1.1 }!
        }
        previous = current
    }
    return previous[new.count].1
}

/// Records every push and decode of the live streams, for `LiveMeasure`.
private final class Recorder: LiveObserver {
    struct Push { var upTo: Int; var at: ContinuousClock.Instant }
    private let state = Mutex<(pushes: [StreamTag: [Push]], decodes: [StreamTag: [LiveDecode]])>(([:], [:]))

    func pushed(_ stream: StreamTag, upTo sample: Int, at: ContinuousClock.Instant) {
        state.withLock { $0.pushes[stream, default: []].append(Push(upTo: sample, at: at)) }
    }

    func decoded(_ stream: StreamTag, _ decode: LiveDecode) {
        state.withLock { $0.decodes[stream, default: []].append(decode) }
    }

    var snapshot: (pushes: [StreamTag: [Push]], decodes: [StreamTag: [LiveDecode]]) { state.withLock { $0 } }
}

/// Partial latency and flicker of one stream, from what the engine sent.
///
/// - What is shown at any moment is the finals so far followed by the current partial, as the
///   engine's words (compared without case or punctuation).
/// - **Word latency**, for word `j` of the stream's final transcript, from the moment the audio at
///   its end reached the engine (the push that carried that sample) to the first display, after
///   that, that shows `j + 1` words (*shown*), and to the first that shows this very word at
///   position `j` (*correct*). Both include the wait for the next hop and the decode; they leave
///   out capture and the core's pump in front of the engine (about 30 ms: a 10 ms pump interval
///   and the 20 ms AGC).
/// - **Staleness**: for each decode, from the arrival of the newest sample it covered to the
///   moment its outputs were sent.
/// - **Flicker**: between consecutive changes of the display, the words of the old one that the
///   new one drops or changes (aligned word by word, so an inserted word does not count the words
///   after it), per 100 final words; and the share of changes that change at least one word shown.
///   The settled words never change, so only the partial is compared.
private struct LiveMeasure {
    var wordShownMs: [Double] = []
    var wordCorrectMs: [Double] = []
    var stalenessMs: [Double] = []
    var decodeMs: [Double] = []
    var changedTotal = 0
    var updates = 0
    var updatesChanging = 0
    var finalWords = 0

    init() {}

    init(pushes: [Recorder.Push], decodes: [LiveDecode]) {
        func arrival(_ sample: Int) -> ContinuousClock.Instant? {
            pushes.first { $0.upTo >= sample }?.at
        }
        var settled: [String] = []
        var wordEnds: [Int] = []
        var display: [String] = []
        var partial: [String] = []
        var snapshots: [(at: ContinuousClock.Instant, words: [String])] = []
        for decode in decodes {
            decodeMs.append(ms(decode.decodeTime))
            if !decode.window.isEmpty, let newest = arrival(decode.window.upperBound) {
                stalenessMs.append(ms(decode.sentAt - newest))
            }
            for output in decode.outputs {
                var next = settled
                var changed = 0
                switch output {
                case .final(let segment):
                    let words = tokens(segment.text)
                    // A final's words are the first words of its decode.
                    for word in decode.decoded.words.prefix(words.count) {
                        wordEnds.append(decode.window.lowerBound + word.end)
                    }
                    // The partial shown is replaced by these words.
                    changed = changedWords(partial, words)
                    partial = []
                    settled += words
                    next = settled
                    finalWords += words.count
                case .partial(let text):
                    let words = tokens(text)
                    changed = changedWords(partial, words)
                    partial = words
                    next = settled + words
                }
                if next != display {
                    updates += 1
                    changedTotal += changed
                    if changed > 0 { updatesChanging += 1 }
                    snapshots.append((decode.sentAt, next))
                }
                display = next
            }
        }
        for (j, end) in wordEnds.enumerated() {
            guard let heard = arrival(end) else { continue }
            let after = snapshots.filter { $0.at >= heard }
            if let shown = after.first(where: { $0.words.count > j }) {
                wordShownMs.append(ms(shown.at - heard))
            }
            if let correct = after.first(where: { $0.words.count > j && $0.words[j] == settled[j] }) {
                wordCorrectMs.append(ms(correct.at - heard))
            }
        }
    }

    var summary: [String: Any] {
        ["word_shown_ms": stats(wordShownMs), "word_correct_ms": stats(wordCorrectMs),
         "staleness_ms": stats(stalenessMs), "decode_ms": stats(decodeMs),
         "changed_words": changedTotal, "updates": updates, "updates_changing_words": updatesChanging,
         "final_words": finalWords,
         "changed_per_100_final_words": finalWords == 0 ? 0 : 100 * Double(changedTotal) / Double(finalWords)]
    }
}

final class AmiHarnessTests: XCTestCase {
    /// The engine's registered rate: what the engine choice measured for this model on these
    /// clips, through FluidAudio's offline path.
    private let measured = ParakeetLiveEngine().wer

    func testTheModelReproducesTheMeasuredWerThroughTheEnginesDecoder() async throws {
        let bench = try bench()
        let model = ParakeetModel.shared
        let loadStarted = ContinuousClock.now
        try await model.load()
        print("parakeet: loaded in \(ContinuousClock.now - loadStarted)")
        var corpus = Wer.Edits()
        var lines: [String] = []
        for clip in try clips(bench) {
            let audio = try read(clip.wav)
            let started = ContinuousClock.now
            let text = try await model.transcribe(audio).text
            let took = ContinuousClock.now - started
            let edits = Wer.score(reference: clip.reference, hypothesis: text)
            corpus = corpus + edits
            let line = "\(clip.wav.lastPathComponent)  \(String(format: "%.1f", Double(audio.count) / 16_000)) s  \(took)  \(edits)"
            print(line)
            lines.append(line)
            try text.write(to: output(bench).appendingPathComponent("offline-\(clip.wav.deletingPathExtension().lastPathComponent).txt"), atomically: true, encoding: .utf8)
        }
        print("corpus  \(corpus)  measured \(measured)")
        lines.append("corpus  \(corpus)  measured \(measured)")
        try lines.joined(separator: "\n").write(to: output(bench).appendingPathComponent("offline.txt"), atomically: true, encoding: .utf8)
        XCTAssertEqual(corpus.reference, 709, "the reference set changed")
        XCTAssertEqual(corpus.wer, measured, accuracy: 0.3)
        let loads = await model.loads
        XCTAssertEqual(loads, 1, "loaded once")
    }

    /// FluidAudio reports `ASRResult.duration` as 0 once audio spans more than one 15 s model
    /// window. The engine never reads it; this shows the reason stands in the pinned version.
    func testFluidAudioReportsNoDurationPastOneModelWindow() async throws {
        let bench = try bench()
        let audio = Array(try read(try XCTUnwrap(clips(bench).first).wav).prefix(20 * 16_000))
        try await ParakeetModel.shared.load()
        let result = try await ParakeetModel.shared.transcribe(audio)
        XCTAssertEqual(audio.count, 20 * 16_000)
        XCTAssertEqual(result.reportedDuration, 0, "if this fails, FluidAudio fixed it: the rule can go")
        XCTAssertFalse(result.words.isEmpty, "the words themselves are timed")
        XCTAssertLessThanOrEqual(result.words.last?.end ?? .max, audio.count)
    }

    func testLivePartialsThroughTheCoreInRealTime() throws {
        let bench = try bench()
        let events = EventLog()
        let data = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-ami-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: data) }
        let session = try InkSession.start(InkConfig(dataDir: data.path, logLevel: "warn"), onEvent: { events.record($0) })
        defer { session.shutdown() }
        let recorder = Recorder()
        let engine = ParakeetLiveEngine(observer: recorder)
        let load = Signal<Bool>()
        Task { load.set((try? await ParakeetModel.shared.load()) != nil) }
        XCTAssertEqual(load.wait(120), true, "models load")
        try session.register(engine)
        let offline = RecordingOffline()
        try session.register(offline)

        var corpus = Wer.Edits()
        var finalPass = Wer.Edits()
        var report: [[String: Any]] = []
        var measures: [LiveMeasure] = []
        for clip in try clips(bench) {
            let before = events.all.count
            let answered = offline.all.count
            try session.command(["cmd": "replay_meeting", "mic": clip.wav.path, "title": "AMI replay"])
            let until = Date().addingTimeInterval(300)
            func ended() -> Bool {
                events.all.dropFirst(before).contains {
                    switch $0 {
                    case .meetingFinished, .meetingFailed: true
                    default: false
                    }
                }
            }
            while !ended(), Date() < until { Thread.sleep(forTimeInterval: 0.1) }
            XCTAssertTrue(ended(), "the replay of \(clip.wav.lastPathComponent) ended")
            let meeting = events.all.dropFirst(before)
            let finals = meeting.compactMap { if case .meetingFinal(let f) = $0, f.channel == .mic { f } else { nil } }
                .sorted { $0.startMs < $1.startMs }
            let text = finals.map(\.text).joined(separator: " ")
            let edits = Wer.score(reference: clip.reference, hypothesis: text)
            corpus = corpus + edits
            try text.write(to: output(bench).appendingPathComponent("live-\(clip.wav.deletingPathExtension().lastPathComponent).txt"), atomically: true, encoding: .utf8)
            let passText = offline.all.dropFirst(answered).joined(separator: " ")
            let passEdits = Wer.score(reference: clip.reference, hypothesis: passText)
            finalPass = finalPass + passEdits
            try passText.write(to: output(bench).appendingPathComponent("final-pass-\(clip.wav.deletingPathExtension().lastPathComponent).txt"), atomically: true, encoding: .utf8)
            print("\(clip.wav.lastPathComponent)  live finals: \(finals.count)  \(edits)  final pass: \(passEdits)")
            report.append(["clip": clip.wav.lastPathComponent, "finals": finals.count, "reference_words": edits.reference,
                           "edits": edits.edits, "final_pass_edits": passEdits.edits,
                           "final_pass_regions": offline.all.count - answered])
        }
        let (pushes, decodes) = recorder.snapshot
        for (tag, streamDecodes) in decodes.sorted(by: { $0.key.number < $1.key.number }) {
            measures.append(LiveMeasure(pushes: pushes[tag] ?? [], decodes: streamDecodes))
        }
        var all = LiveMeasure()
        for m in measures {
            all.wordShownMs += m.wordShownMs
            all.wordCorrectMs += m.wordCorrectMs
            all.stalenessMs += m.stalenessMs
            all.decodeMs += m.decodeMs
            all.changedTotal += m.changedTotal
            all.updates += m.updates
            all.updatesChanging += m.updatesChanging
            all.finalWords += m.finalWords
        }
        let summary: [String: Any] = [
            "clips": report, "corpus_wer": corpus.wer, "corpus_edits": corpus.edits, "reference_words": corpus.reference,
            "final_pass_wer": finalPass.wer, "final_pass_edits": finalPass.edits, "final_pass_inputs": offline.received,
            "measured_offline_wer": measured, "config": String(describing: engine.config), "live": all.summary,
        ]
        let json = try JSONSerialization.data(withJSONObject: summary, options: [.prettyPrinted, .sortedKeys])
        try json.write(to: output(bench).appendingPathComponent("live.json"))
        print(String(decoding: json, as: UTF8.self))
        print("live finals  \(corpus)   final pass  \(finalPass)   measured \(measured)")
        XCTAssertEqual(corpus.reference, 709)
        XCTAssertLessThan(corpus.wer, measured + 3, "the live finals lose words the offline pass hears")
    }
}

/// The live scheme's decisions on the real model, without the core or real time: each clip fed a
/// block at a time with every decode awaited, for trying the scheme's settings quickly. Only with
/// INK_LIVE_SWEEP=1 as well. Latency here is audio time (the decode takes none), so it reads lower
/// than the real-time harness's.
private extension LiveOutput {
    var isFinal: Bool { if case .final = self { true } else { false } }
}

final class LiveSchemeSweepTests: XCTestCase {
    func testTheSchemeOnTheRealModel() async throws {
        let bench = try bench()
        guard ProcessInfo.processInfo.environment["INK_LIVE_SWEEP"] == "1" else {
            throw XCTSkip("set INK_LIVE_SWEEP=1 as well")
        }
        let model = ParakeetModel.shared
        try await model.load()
        let audio = try clips(bench).map { ($0, try read($0.wav)) }
        for hide in [0, LiveWindowConfig().hideNewest, 5_120] {
            var config = LiveWindowConfig()
            config.hideNewest = hide
            var corpus = Wer.Edits()
            var changed = 0, finalWords = 0, updates = 0, changing = 0
            var latency: [Double] = []
            for (clip, samples) in audio {
                var live = LiveWindow(config: config)
                var settled: [String] = []
                var partial: [String] = []
                var counts: [(received: Int, count: Int)] = []
                var ends: [Int] = []
                var text: [String] = []
                func take(_ outputs: [LiveOutput], _ window: Window, _ decoded: DecodedWindow, _ received: Int) {
                    for output in outputs {
                        let next: [String]
                        switch output {
                        case .final(let seg):
                            next = tokens(seg.text)
                            ends += decoded.words.prefix(next.count).map { window.start + $0.end }
                            text.append(seg.text)
                            finalWords += next.count
                        case .partial(let p):
                            next = tokens(p)
                        }
                        let c = changedWords(partial, next)
                        if next != partial || output.isFinal {
                            updates += 1
                            changed += c
                            if c > 0 { changing += 1 }
                        }
                        if output.isFinal {
                            settled += next
                            partial = []
                        } else {
                            partial = next
                        }
                        counts.append((received, settled.count + partial.count))
                    }
                }
                var t = 0
                while t < samples.count {
                    let n = min(1_600, samples.count - t)
                    live.append(Array(samples[t..<t + n]))
                    t += n
                    if live.wantsDecode {
                        let w = live.takeWindow()
                        let d = try await model.decode(w.samples)
                        take(live.apply(d, of: w), w, d, t)
                    }
                }
                if let w = live.takeLastWindow() {
                    let d = try await model.decode(w.samples)
                    take(live.applyLast(d, of: w), w, d, t)
                }
                // Audio time from a word's end to the first display, from then on, of j + 1 words.
                for (j, end) in ends.enumerated() {
                    if let shown = counts.first(where: { $0.received >= end && $0.count > j }) {
                        latency.append(Double(shown.received - end) / 16)
                    }
                }
                corpus = corpus + Wer.score(reference: clip.reference, hypothesis: text.joined(separator: " "))
            }
            let sorted = latency.sorted()
            print("sweep  hideNewest \(hide)  finals \(corpus)  changed/100 final words \(String(format: "%.0f", 100 * Double(changed) / Double(max(finalWords, 1))))  updates changing words \(String(format: "%.1f", 100 * Double(changing) / Double(max(updates, 1)))) %  audio latency ms p50 \(String(format: "%.0f", percentile(sorted, 50))) p90 \(String(format: "%.0f", percentile(sorted, 90)))")
        }
    }
}
