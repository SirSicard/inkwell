// Marks for scripts/idle-budget.sh (the shell budget, I7). Off unless INK_MEASURE is set, and
// then only lines on stdout: which state was asked for, when the core is ready, when the dictation
// model is warm, and on SIGUSR1 the frames drawn so far. Event-driven like the rest of the shell:
// the measurement adds no timer to what it measures.
//
// INK_MEASURE names the state to hold: "idle", or "live" (the ink drawing at 60 fps with no audio,
// which the renderer honours once it exists; until then nothing draws and the script says the
// live budget was not measured).
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

    /// A measurement when INK_MEASURE is set, else nil.
    static func fromEnvironment(_ environment: [String: String]) -> Measurement? {
        guard let state = environment["INK_MEASURE"], !state.isEmpty else { return nil }
        return Measurement(state: state)
    }

    /// Starts marking: the state now, the frame count on every SIGUSR1.
    func start(windowVisible: @escaping @MainActor () -> Bool) {
        mark("start state=\(state) pid=\(getpid())")
        signal(SIGUSR1, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
        source.setEventHandler {
            MainActor.assumeIsolated {
                self.mark("frames=\(InkRenderer.frames.count) visible=\(windowVisible())")
            }
        }
        source.resume()
        signalSource = source
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

    private func mark(_ line: String) {
        // Unbuffered, so the script sees each mark as it happens even with stdout on a file.
        FileHandle.standardOutput.write(Data("mark \(line)\n".utf8))
    }
}
