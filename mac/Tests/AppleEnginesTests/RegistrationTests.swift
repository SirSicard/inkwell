// The Apple engines registered with a running core: live partials through ink_register_engine,
// called on the core's worker threads and answering through ink_engine_complete and
// ink_stream_event; polish registered only while Apple Intelligence is available; missing models
// reported, never downloaded. No model runs here: the decoders are scripted.
@testable import AppleEngines
import Foundation
import InkBridge
import Synchronization
import XCTest

/// A word every 0.4 s of a window, up to its end: unbroken speech, so a replay gets growing
/// partials, and finals from the length bound and the stream's end, without a model.
private struct Chatter: WindowDecoder {
    func decode(_ samples: [Float]) async throws(ParakeetError) -> DecodedWindow {
        let n = max(0, (samples.count - 4_800) / 6_400)
        return DecodedWindow(words: (0..<n).map {
            TimedWord(text: "synthetic", start: $0 * 6_400, end: $0 * 6_400 + 4_800)
        })
    }
}

/// A loaded model that hears nothing.
private struct Silent: ParakeetBackend {
    func transcribe(_ samples: [Float]) async throws(ParakeetError) -> Transcribed {
        Transcribed(text: "", words: [], reportedDuration: 0)
    }
}

/// Which threads pushed.
private final class Threads: LiveObserver {
    private let names = Mutex<Set<String>>([])
    private let main = Atomic<Bool>(false)
    var seen: Set<String> { names.withLock { $0 } }
    var onMain: Bool { main.load(ordering: .relaxed) }

    func pushed(_ stream: StreamTag, upTo sample: Int, at: ContinuousClock.Instant) {
        let name = currentThreadName()
        names.withLock { _ = $0.insert(name) }
        if Thread.isMainThread { main.store(true, ordering: .relaxed) }
    }

    func decoded(_ stream: StreamTag, _ decode: LiveDecode) {}
}

final class RegistrationTests: XCTestCase {
    func testTheLiveEngineGivesAReplayedMeetingItsPartialsFromTheCoresWorker() throws {
        let core = try TestCore.start()
        defer { core.stop() }
        let threads = Threads()
        try core.session.register(ParakeetLiveEngine(decoder: Chatter(), id: "parakeet-scripted", observer: threads))
        XCTAssertNotNil(core.events.wait(5) { if case .engineRegistered(let e) = $0, e.kind == .streaming { e } else { nil } })

        let mic = fixtures.appendingPathComponent("ami/IS1009a-mic.wav").path
        let far = fixtures.appendingPathComponent("ami/IS1009a-far.wav").path
        try core.session.command(["cmd": "replay_meeting", "mic": mic, "far": far, "pacing": "fast"])
        let ended = core.events.wait(120) {
            switch $0 {
            case .meetingFinished, .meetingFailed: true
            default: nil
            }
        }
        XCTAssertNotNil(ended, "\(core.events.all)")

        let all = core.events.all
        for side in [Channel.mic, .far] {
            let partials = all.compactMap { if case .meetingPartial(let p) = $0, p.channel == side { p.text } else { nil } }
            let finals = all.compactMap { if case .meetingFinal(let f) = $0, f.channel == side { f } else { nil } }
            XCTAssertTrue(partials.contains { $0.hasPrefix("synthetic") }, "\(side) partials: \(partials)")
            XCTAssertFalse(finals.isEmpty, "\(side) has live finals")
            XCTAssertTrue(finals.allSatisfy { $0.endMs >= $0.startMs && $0.endMs <= 31_000 })
        }
        XCTAssertEqual(threads.seen, ["ink-meeting"], "pushed from the core's worker only")
        XCTAssertFalse(threads.onMain)
    }

    /// Parakeet also registers for both finals, with the rates measured for it: worse than
    /// Qwen3-ASR's, so the router uses it only while Qwen3-ASR is not installed. With no registry
    /// model installed here, it serves every speech job; `engine.route` is where the shell sees
    /// that.
    func testParakeetRegistersAsTheFinalsFallback() async throws {
        let core = try TestCore.start()
        defer { core.stop() }
        let loaded = ParakeetModel(loader: { Silent() })
        let report = await AppleEngines(session: core.session, parakeet: loaded).register()
        XCTAssertEqual(report.livePartials, .registered)
        XCTAssertEqual(report.finals, .registered)
        let offline = try XCTUnwrap(core.events.wait(5) {
            if case .engineRegistered(let e) = $0, e.kind == .offline { e } else { nil }
        })
        XCTAssertEqual(offline.id, ParakeetOfflineEngine.fallbackID)
        // The core keeps rates as 32-bit floats: compared to a tenth.
        XCTAssertEqual(
            Set(offline.jobs.map { "\($0.job.rawValue) \(String(format: "%.1f", $0.wer))" }),
            ["dictation_final 6.7", "meeting_final 23.4"])
        for job in [Job.dictationFinal, .meetingFinal] {
            try core.session.command(["cmd": "engine.route", "job": job.rawValue])
            let routed = try XCTUnwrap(core.events.wait(5) {
                if case .engineRouted(let r) = $0, r.job == job { r } else { nil }
            })
            XCTAssertEqual(routed.id, ParakeetOfflineEngine.fallbackID)
            XCTAssertEqual(routed.source, .shell)
        }
        // And the live words: Parakeet alone serves every speech job, which is what lets the first
        // run recommend it (with voice detection) and leave Qwen3-ASR optional.
        try core.session.command(["cmd": "engine.route", "job": Job.livePartials.rawValue])
        let live = try XCTUnwrap(core.events.wait(5) {
            if case .engineRouted(let r) = $0, r.job == .livePartials { r } else { nil }
        })
        XCTAssertEqual(live.id, "fluidaudio-parakeet-tdt-0.6b-v3")
        XCTAssertEqual(live.source, .shell)
    }

    func testMissingModelsAreReportedAndNothingIsRegistered() async throws {
        let core = try TestCore.start()
        defer { core.stop() }
        let missing = ParakeetModel(loader: { () throws(ParakeetError) -> any ParakeetBackend in
            throw .modelMissing
        })
        let report = await AppleEngines(session: core.session, parakeet: missing).register()
        XCTAssertEqual(report.livePartials, .modelMissing)
        XCTAssertEqual(report.finals, .modelMissing)
        XCTAssertTrue(core.registered.allSatisfy { $0.kind == .llm }, "\(core.registered)")
    }

    /// Parakeet not downloaded at launch is registered when its download finishes, not at the next
    /// launch: once, and only in a session whose shell registers the Apple engines.
    func testParakeetIsRegisteredOnceWhenItsDownloadFinishes() async throws {
        let core = try TestCore.start()
        defer { core.stop() }
        let downloaded = Counter()
        let model = ParakeetModel(loader: { () throws(ParakeetError) -> any ParakeetBackend in
            if downloaded.value == 0 { throw .modelMissing }
            return Silent()
        })
        func finished(_ id: String, ok: Bool) throws -> InkEvent {
            try InkEvent.decode(Data(#"{"type":"model.update_finished","id":"\#(id)","next":"\#(id)","ok":\#(ok),"no_model_warm":false}"#.utf8))
        }
        let parakeetInstalled = try finished(ParakeetFiles.rowID, ok: true)

        // A shell that registers no Apple engines (it never called `register`): nothing.
        AppleEngines(session: core.session, parakeet: ParakeetModel(loader: { Silent() })).handle(parakeetInstalled)

        let engines = AppleEngines(session: core.session, parakeet: model)
        let report = await engines.register()
        XCTAssertEqual(report.livePartials, .modelMissing, "not downloaded yet")
        downloaded.add()
        // Another model's download, or Parakeet's failing: nothing.
        engines.handle(try finished("qwen3-asr-1.7b-q8", ok: true))
        engines.handle(try finished(ParakeetFiles.rowID, ok: false))
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertTrue(core.registered.allSatisfy { $0.kind == .llm }, "\(core.registered)")

        engines.handle(parakeetInstalled)
        let until = ContinuousClock.now + .seconds(5)
        while core.registered.filter({ $0.kind != .llm }).count < 2, ContinuousClock.now < until {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(
            Set(core.registered.filter { $0.kind != .llm }.map(\.kind)), [.streaming, .offline],
            "live partials and the finals' fallback")
        // Its download reported twice, or registration asked for again: registered once, and a
        // later ask reads registered (the core would refuse the same ids a second time).
        engines.handle(parakeetInstalled)
        try await Task.sleep(for: .milliseconds(300))
        let again = await engines.register()
        XCTAssertEqual(again.livePartials, .registered)
        XCTAssertEqual(again.finals, .registered)
        XCTAssertEqual(core.registered.filter { $0.kind != .llm }.count, 2)
    }

    func testPolishIsRegisteredOnlyWhileAppleIntelligenceIsAvailable() throws {
        let core = try TestCore.start()
        defer { core.stop() }
        let engines = AppleEngines(session: core.session)
        let off = AppleIntelligence.unavailable(.notEnabled)
        XCTAssertEqual(engines.syncPolish(availability: off), .unavailable(code: 2))
        XCTAssertTrue(core.registered.isEmpty)

        XCTAssertEqual(engines.syncPolish(availability: .available), .registered)
        let registered = core.events.wait(5) { if case .engineRegistered(let e) = $0 { e } else { nil } }
        XCTAssertEqual(registered?.kind, .llm)
        XCTAssertEqual(registered?.id, "apple-foundation-models")
        XCTAssertEqual(registered?.jobs, [])
        // Available again: nothing changes.
        XCTAssertEqual(engines.syncPolish(availability: .available), .registered)

        // Turned off: let go of.
        XCTAssertEqual(engines.syncPolish(availability: off), .unavailable(code: 2))
        XCTAssertNotNil(core.events.wait(5) { if case .engineUnregistered(let e) = $0 { e.id } else { nil } })
        XCTAssertEqual(core.registered.count, 1)
    }
}
