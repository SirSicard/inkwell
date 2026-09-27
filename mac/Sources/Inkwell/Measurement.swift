// Marks for scripts/idle-budget.sh (the shell budget, I7). Off unless INK_MEASURE is set, and
// then only lines on stdout: which state was asked for, when the core is ready, when the dictation
// model is warm, and on SIGUSR1 the frames drawn so far. Event-driven like the rest of the shell:
// the measurement adds no timer to what it measures.
//
// INK_MEASURE names the state to hold: "idle", or "live": the ink held in the meeting state with no
// audio, so the Drop and the window's rail both draw at 60 fps (a meeting with the window open,
// the most the ink draws at once). Every frame either surface presents is counted, so "frames"
// grows by about 120 a second while live. Live also records the GPU time of each frame, and the
// SIGUSR1 mark adds its spread since the previous mark.
import AppKit
import Foundation
import InkBridge
import InkRenderer

@MainActor
final class Measurement {
    /// The state INK_MEASURE asked for.
    let state: String
    private var signalSource: DispatchSourceSignal?

    private init(state: String) {
        self.state = state
    }

    deinit {
        // A released source that was never cancelled would stay armed with nothing behind it.
        signalSource?.cancel()
    }

    /// The ink state the measurement holds: the meeting state for "live", none otherwise.
    var heldInk: InkState? {
        state == "live" ? .meeting : nil
    }

    /// A measurement when INK_MEASURE is set, else nil.
    static func fromEnvironment(_ environment: [String: String]) -> Measurement? {
        guard let state = environment["INK_MEASURE"], !state.isEmpty else { return nil }
        return Measurement(state: state)
    }

    /// Starts marking: the state now, the frame count on every SIGUSR1.
    func start(windowVisible: @escaping @MainActor () -> Bool) {
        mark("start state=\(state) pid=\(getpid())")
        if heldInk != nil {
            InkRenderer.gpuTimes.start()
        }
        signal(SIGUSR1, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
        // Weak: the source is this object's, so a strong capture would keep both alive forever.
        source.setEventHandler { [weak self] in
            MainActor.assumeIsolated {
                self?.mark("frames=\(InkRenderer.frames.count) visible=\(windowVisible())\(Self.gpuSpread())")
            }
        }
        source.resume()
        signalSource = source
    }

    /// The GPU time per frame since the last mark, when it is recorded: " gpu_frames=N
    /// gpu_ms_p50=… gpu_ms_p95=… gpu_ms_max=…" (after `visible`, which the script reads first).
    private static func gpuSpread() -> String {
        guard InkRenderer.gpuTimes.isRecording else { return "" }
        guard let s = InkRenderer.gpuTimes.drain() else { return " gpu_frames=0" }
        return String(format: " gpu_frames=%d gpu_ms_p50=%.3f gpu_ms_p95=%.3f gpu_ms_max=%.3f", s.frames, s.p50, s.p95, s.max)
    }

    /// Marks the events the script waits for: types, jobs, model ids, and a failed warm's message
    /// (a core error, which never quotes the user's words, I5). Never a payload that carries them.
    func note(_ batch: [InkEvent]) {
        for event in batch {
            switch event {
            case .coreReady(let ready):
                mark("event=core.ready version=\(ready.version)")
            case .modelWarmed(let warmed):
                mark("event=model.warmed job=\(warmed.job.rawValue) id=\(warmed.id)")
            case .modelWarmFailed(let failed):
                let message = failed.message.replacingOccurrences(of: "\n", with: " ")
                mark("event=model.warm_failed job=\(failed.job.rawValue) message=\(message)")
            case .coreStopped:
                mark("event=core.stopped")
            default:
                break
            }
        }
    }

    /// The ink's shader finished compiling (off the main thread), and how long it took.
    func inkReady(_ outcome: InkPipelineLoader.Outcome, took: Duration) {
        switch outcome {
        case .success:
            mark(String(format: "ink ready compile_ms=%.1f", took / .milliseconds(1)))
        case .failure(let failure):
            mark("ink failed \(failure.description.replacingOccurrences(of: "\n", with: " "))")
        }
    }

    /// The main window was asked to show (at launch, from the menu, or by a second copy).
    func windowShown() {
        mark("window shown")
    }

    private func mark(_ line: String) {
        // Unbuffered, so the script sees each mark as it happens even with stdout on a file.
        FileHandle.standardOutput.write(Data("mark \(line)\n".utf8))
    }
}
