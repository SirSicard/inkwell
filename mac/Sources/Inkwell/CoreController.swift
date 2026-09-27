// Starts and stops the core, and feeds its events to the store.
//
//   core event thread ──push──▶ EventRelay ──one hop per batch──▶ main actor: store.apply(batch)
//
// The shell only sends commands and renders the store (architecture rule 1). Nothing here polls.
import Foundation
import InkBridge
import os

@MainActor
final class CoreController {
    /// Everything the screens show about the core.
    let store = CoreStore()
    /// What the screens show of the library (Today, Library, a record), asked for by query.
    let library = LibraryModel()
    /// Sees every batch after the store (the measurement marks), when set.
    var observer: ((_ batch: [InkEvent]) -> Void)?

    private var session: InkSession?
    private var stopping = false
    private let log = Logger(subsystem: "com.inkwell.app", category: "core")

    /// Whether the core is running (started, and not yet told to stop).
    var isRunning: Bool { session != nil }

    init() {
        library.send = { [weak self] json in self?.send(json: json) }
    }

    /// Starts the core with the library at `dataDirectory`. A failure is shown through the store.
    func start(environment: [String: String] = ProcessInfo.processInfo.environment) {
        guard session == nil, !stopping else { return }
        let relay = EventRelay { [weak self] batch in
            self?.received(batch)
        }
        do {
            let data = try DataLocation.dataDirectory(environment: environment)
            let models = try DataLocation.modelsDirectory(environment: environment)
            try DataLocation.create(data)
            let config = InkConfig(dataDir: data.path, modelsDir: models?.path)
            session = try InkSession.start(config, onEvent: { relay.push($0) })
        } catch {
            // The error names a path or a core status, never anything the user said.
            log.error("the core did not start: \(String(describing: error), privacy: .public)")
            store.startFailed("The core did not start: \(error)")
        }
    }

    /// Queues a command (inkwell.h lists them); its outcome arrives as events. A command the core
    /// refuses to queue (unreadable, or the core is stopping) never ran, and is logged by name.
    func send(_ fields: [String: String]) {
        guard let session else { return }
        do {
            try session.command(fields)
        } catch {
            log.error("the core refused a \(fields["cmd"] ?? "?", privacy: .public) command: \(String(describing: error), privacy: .public)")
        }
    }

    /// Queues a command already written as JSON (the library's queries carry numbers and objects).
    func send(json: String) {
        guard let session else { return }
        do {
            try session.command(json)
        } catch {
            // The command's name only: a query can carry the user's search words.
            let name = (try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])?["cmd"] as? String
            log.error("the core refused a \(name ?? "?", privacy: .public) command: \(String(describing: error), privacy: .public)")
        }
    }

    /// Stops the core off the main thread (it waits for its workers and unloads every model), then
    /// calls `done` on the main thread. When `done` runs, no event arrives any more.
    ///
    /// `done` goes back as a run-loop block, not a main-queue block: Quit waits for it inside
    /// AppKit's nested run loop (terminateLater), and when Quit itself was called from a
    /// main-queue block, that loop cannot run another main-queue block until the first returns.
    /// A run-loop block runs in the nested loop either way.
    func stop(then done: @escaping @MainActor @Sendable () -> Void) {
        guard let session else {
            done()
            return
        }
        self.session = nil
        stopping = true
        DispatchQueue.global(qos: .userInitiated).async {
            session.shutdown()
            let main = CFRunLoopGetMain()
            CFRunLoopPerformBlock(main, CFRunLoopMode.commonModes.rawValue) {
                MainActor.assumeIsolated { done() }
            }
            CFRunLoopWakeUp(main)
        }
    }

    private func received(_ batch: [InkEvent]) {
        store.apply(batch)
        library.apply(batch)
        // Keep the dictation model warm from the start: the first dictation of the day is as quick
        // as any other (the shell budget, I7, is measured with it warm).
        if batch.contains(where: { if case .coreReady = $0 { true } else { false } }),
            case .ready = store.status
        {
            send(["cmd": "model.warm", "job": Job.dictationFinal.rawValue])
        }
        observer?(batch)
    }
}
