// One display link for every live ink. With the Drop, the window's orb and its edge glow all live,
// one callback per vsync steps and draws them all, instead of a link per view each waking the main
// thread. Each view's schedule (InkSchedule) still decides whether it takes part: a view joins when
// its ink goes live on screen and leaves when it settles, is covered or cannot draw. The link exists
// only while at least one view is on it (architecture rule 9: nothing ticks while idle).
import AppKit
import QuartzCore

/// A view the clock steps once per vsync: the orb (InkView) or the edge glow (GlowEdgeView).
@MainActor
protocol InkClockClient: AnyObject {
    /// The window it is in: the link starts on that window's screen.
    var window: NSWindow? { get }
    /// One vsync, at the display link's timestamp.
    func clockTicked(at now: CFTimeInterval)
}

/// The display link the live inks share. Main thread only.
@MainActor
public final class InkClock: NSObject {
    /// The process's clock, which every InkView uses unless given another.
    public static let shared = InkClock()

    private struct Client {
        weak var view: (any InkClockClient)?
    }

    private var clients: [Client] = []
    private var link: CADisplayLink?

    /// Views on the clock now.
    var clientCount: Int { clients.count { $0.view != nil } }
    /// Display links the clock holds: 1 while a view is on it, 0 otherwise (and 0 on a Mac with no
    /// screen, where no view is on screen to draw).
    var linkCount: Int { link == nil ? 0 : 1 }

    override init() {
        super.init()
        // A link belongs to one screen; when the screens change, it moves to the main one.
        NotificationCenter.default.addObserver(
            self, selector: #selector(screensChanged(_:)),
            name: NSApplication.didChangeScreenParametersNotification, object: nil)
    }

    func contains(_ view: any InkClockClient) -> Bool {
        clients.contains { $0.view === view }
    }

    /// Puts `view` on the clock; the first view starts the link, on its own screen.
    func add(_ view: any InkClockClient) {
        guard !contains(view) else { return }
        clients.append(Client(view: view))
        if link == nil {
            startLink(on: view.window?.screen)
        }
    }

    /// Takes `view` off the clock; the last view to leave stops the link.
    func remove(_ view: any InkClockClient) {
        clients.removeAll { $0.view === view || $0.view == nil }
        if clients.isEmpty {
            stopLink()
        }
    }

    /// One vsync: every view on the clock steps and draws, in the order they joined.
    func tick(at timestamp: CFTimeInterval) {
        clients.removeAll { $0.view == nil }
        // A view may leave during its own tick; the others still get theirs.
        for client in clients {
            client.view?.clockTicked(at: timestamp)
        }
        if clients.isEmpty {
            stopLink()
        }
    }

    @objc private func fire(_ displayLink: CADisplayLink) {
        tick(at: displayLink.timestamp)
    }

    private func startLink(on screen: NSScreen?) {
        guard link == nil, let screen = screen ?? NSScreen.main ?? NSScreen.screens.first else { return }
        let displayLink = screen.displayLink(target: self, selector: #selector(fire(_:)))
        displayLink.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 60, preferred: 60)
        displayLink.add(to: .main, forMode: .common)
        link = displayLink
    }

    private func stopLink() {
        link?.invalidate()
        link = nil
    }

    // AppKit posts it on the main thread; the hop covers a sender that ever does not.
    @objc private nonisolated func screensChanged(_ note: Notification) {
        let restart: @MainActor @Sendable () -> Void = { [weak self] in
            guard let self, self.link != nil else { return }
            self.stopLink()
            self.startLink(on: nil)
        }
        if Thread.isMainThread {
            MainActor.assumeIsolated { restart() }
        } else {
            DispatchQueue.main.async { MainActor.assumeIsolated { restart() } }
        }
    }
}
