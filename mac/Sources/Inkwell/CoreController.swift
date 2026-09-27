// Starts and stops the core, and feeds its events to the store.
//
//   core event thread ──push──▶ EventRelay ──one hop per batch──▶ main actor: store.apply(batch)
//
// The shell only sends commands and renders the store (architecture rule 1). Nothing here polls.
import AppKit
import AppleEngines
import Foundation
import InkBridge
import os
import Synchronization

@MainActor
final class CoreController {
    /// Everything the screens show about the core.
    let store = CoreStore()
    /// The Live, Owed and Settings screens' models and the first-run state (fed after the store).
    private(set) lazy var screens = ScreenModels(
        send: { [weak self] in self?.send($0) },
        dataDirectory: try? DataLocation.dataDirectory(environment: ProcessInfo.processInfo.environment),
        modelsDirectory: try? DataLocation.modelsDirectory(environment: ProcessInfo.processInfo.environment))
    /// Sees every batch after the store (the measurement marks), when set.
    var observer: ((_ batch: [InkEvent]) -> Void)?

    private var session: InkSession?
    private var stopping = false
    /// The Apple engines of this session, and what forwards the core's events to them.
    private var apple: AppleEngines?
    private let appleEvents = AppleEventForwarder()
    /// Re-registers polish when Apple Intelligence comes or goes mid-session.
    private var intelligence: AppleIntelligenceWatch?
    private var activation: NSObjectProtocol?
    private let log = Logger(subsystem: "com.inkwell.app", category: "core")

    /// Whether the core is running (started, and not yet told to stop).
    var isRunning: Bool { session != nil }

    /// Starts the core with the library at `dataDirectory`. A failure is shown through the store.
    func start(environment: [String: String] = ProcessInfo.processInfo.environment) {
        guard session == nil, !stopping else { return }
        let relay = EventRelay { [weak self] batch in
            self?.received(batch)
        }
        let forward = appleEvents
        do {
            let data = try DataLocation.dataDirectory(environment: environment)
            let models = try DataLocation.modelsDirectory(environment: environment)
            try DataLocation.create(data)
            let config = InkConfig(dataDir: data.path, modelsDir: models?.path)
            let started = try InkSession.start(config, onEvent: {
                relay.push($0)
                forward.handle($0)
            })
            session = started
            let engines = AppleEngines(session: started)
            apple = engines
            appleEvents.attach(engines)
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

    /// Queues one of the screens' commands (see `CoreCommand`).
    func send(_ command: CoreCommand) {
        guard let session else { return }
        do {
            try session.command(command.json)
        } catch {
            log.error("the core refused a \(command.name, privacy: .public) command: \(String(describing: error), privacy: .public)")
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
        intelligence?.stop()
        intelligence = nil
        appleEvents.attach(nil)
        apple = nil
        if let activation {
            NotificationCenter.default.removeObserver(activation)
        }
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
        screens.apply(batch)
        // Keep the dictation model warm from the start: the first dictation of the day is as quick
        // as any other (the shell budget, I7, is measured with it warm).
        if batch.contains(where: { if case .coreReady = $0 { true } else { false } }),
            case .ready = store.status
        {
            send(["cmd": "model.warm", "job": Job.dictationFinal.rawValue])
            registerAppleEngines()
        }
        if batch.contains(where: { if case .dictationStarted = $0 { true } else { false } }) {
            // A take is a moment Apple Intelligence may have changed since the last look.
            intelligence?.recheck()
        }
        observer?(batch)
    }

    /// Registers the Apple engines (Parakeet: seconds on the first launch, nothing downloaded;
    /// polish while Apple Intelligence is available), then watches Apple Intelligence so polish
    /// is registered or let go of when it changes mid-session, and re-checks it whenever the app
    /// becomes active.
    private func registerAppleEngines() {
        guard let engines = apple, intelligence == nil else { return }
        let polish = screens.polish
        intelligence = AppleIntelligenceWatch { [weak engines] availability in
            guard let engines else { return }
            let state = engines.syncPolish(availability: availability)
            Task { @MainActor in polish.appleEnginesReported(state) }
        }
        activation = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.intelligence?.recheck()
                self?.screens.appBecameActive()
            }
        }
        Task {
            let report = await engines.register()
            polish.appleEnginesReported(report.polish)
            #if DEBUG
                // After the engines, so a replayed meeting has its live words.
                if let replay = ReplayOnLaunch.command(from: ProcessInfo.processInfo.environment) {
                    self.send(replay)
                }
            #endif
            self.log.notice("Apple engines: live \(String(describing: report.livePartials), privacy: .public), finals \(String(describing: report.finals), privacy: .public), polish \(String(describing: report.polish), privacy: .public)")
        }
    }
}

/// Hands the core's events to the Apple engines, from the core's event thread (it returns at once:
/// a take starting only prewarms polish). The engines are attached once the session has started.
final class AppleEventForwarder: Sendable {
    private let engines = Mutex<AppleEngines?>(nil)

    func attach(_ engines: AppleEngines?) {
        self.engines.withLock { $0 = engines }
    }

    func handle(_ event: InkEvent) {
        engines.withLock { $0 }?.handle(event)
    }
}
