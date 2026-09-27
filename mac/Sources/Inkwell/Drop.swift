// The Drop: the surface for everything live. A small paper panel near the bottom of the screen,
// with the ink on its left and two lines beside it, shown while something is live and hidden when
// idle. It never takes focus: while it shows, keystrokes and clicks elsewhere go where the user
// put them, and a click on the Drop itself does not activate Inkwell (mac/DROP-FOCUS-CHECKLIST.md
// is the check by hand).
//
// Paper in both themes, like the rail: ink on a dark page would vanish.
import AppKit
import InkBridge
import InkRenderer
import Observation
import QuartzCore

/// The Drop's window: a non-activating panel that cannot become key or main.
final class DropPanel: NSPanel {
    init() {
        super.init(contentRect: NSRect(origin: .zero, size: DropLayout.size),
                   styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: true)
        isFloatingPanel = true
        // Above other apps' windows and their floating panels, the Dock's level included.
        level = .statusBar
        // Inkwell is never the active app while the Drop shows, so this must be off or it would
        // hide at once.
        hidesOnDeactivate = false
        becomesKeyOnlyIfNeeded = true
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        isReleasedWhenClosed = false
        isMovable = false
        animationBehavior = .none
        // On every Space and beside full-screen apps; not part of Cmd-` window cycling.
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        isExcludedFromWindowsMenu = true
        // The paper surface looks the same in both themes; so does its text.
        appearance = NSAppearance(named: .aqua)
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// The Drop's measures, from the design canvas's mock: a 16 pt corner, a hairline rule, the ink
/// on the left at the panel's full height.
enum DropLayout {
    static let size = NSSize(width: 320, height: 84)
    static let inkWidth: CGFloat = 96
    static let cornerRadius: CGFloat = 16
    /// Above the bottom of the visible screen (the Dock's top when it shows).
    static let bottomMargin: CGFloat = 28
}

/// Shows and hides the Drop as the ink's state changes.
@MainActor
final class DropController {
    private let ink: ShellInk
    private let panel = DropPanel()
    private let content = DropContentView()
    private(set) var isShown = false

    /// The state the Drop's ink shows.
    var inkState: InkState { content.inkView.state }
    /// Whether the Drop is the key window (never, by design).
    var panelIsKey: Bool { panel.isKeyWindow }

    init(ink: ShellInk) {
        self.ink = ink
        panel.contentView = content
        update()
        observe()
    }

    /// Brings the Drop in line with the ink's state.
    func update() {
        let state = ink.state
        if state.isLive {
            content.show(ink.dropText)
            content.inkView.state = state
            if !isShown {
                place()
                panel.orderFrontRegardless()
                isShown = true
                content.inkView.updateVisibility()
            }
        } else if isShown {
            // Out first: the ink stops without drawing a last frame nobody would see.
            panel.orderOut(nil)
            isShown = false
            content.inkView.updateVisibility()
            content.inkView.state = .idle
        }
    }

    /// Re-reads the state after every change the store or a held state makes: the change is
    /// handed to the next turn of the main queue, once the store has finished applying it.
    private func observe() {
        withObservationTracking {
            _ = ink.state
            _ = ink.dropText
        } onChange: { [weak self] in
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    self?.update()
                    self?.observe()
                }
            }
        }
    }

    /// Bottom centre of the screen the user is working on (the one with the key window).
    private func place() {
        guard let screen = NSScreen.main ?? NSScreen.screens.first else { return }
        let visible = screen.visibleFrame
        panel.setFrameOrigin(NSPoint(x: (visible.midX - DropLayout.size.width / 2).rounded(),
                                     y: visible.minY + DropLayout.bottomMargin))
    }
}

/// The Drop's content: paper, the ink, two lines.
final class DropContentView: NSView {
    let inkView = InkView(frame: NSRect(x: 0, y: 0, width: DropLayout.inkWidth, height: DropLayout.size.height))
    private let title = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")
    private let inkHolder = NSView()

    private static let paper = Palette.paper.nsColor
    private static let rule = Palette.ink.nsColor.withAlphaComponent(0.12)

    init() {
        super.init(frame: NSRect(origin: .zero, size: DropLayout.size))
        wantsLayer = true
        guard let layer else { return }
        layer.backgroundColor = Self.paper.cgColor
        layer.cornerRadius = DropLayout.cornerRadius
        layer.cornerCurve = .continuous
        layer.masksToBounds = true
        layer.borderWidth = 1
        layer.borderColor = Self.rule.cgColor

        // The ink zone's own paper (grain, fibres, a vignette) fades into the panel's flat paper
        // over its right edge, so no seam shows where the zone ends. The ink itself stays clear of
        // that edge (the shader's fence).
        inkHolder.frame = inkView.frame
        inkHolder.wantsLayer = true
        let fade = CAGradientLayer()
        fade.frame = inkHolder.bounds
        fade.startPoint = CGPoint(x: 0, y: 0.5)
        fade.endPoint = CGPoint(x: 1, y: 0.5)
        fade.colors = [NSColor.black.cgColor, NSColor.black.cgColor, NSColor.clear.cgColor]
        fade.locations = [0, 0.82, 1]
        inkHolder.layer?.mask = fade
        inkHolder.addSubview(inkView)
        addSubview(inkHolder)

        title.font = .systemFont(ofSize: 12, weight: .medium)
        title.textColor = Palette.muted.nsColor
        detail.font = .systemFont(ofSize: 14)
        detail.textColor = Palette.ink.nsColor
        detail.lineBreakMode = .byTruncatingTail
        let lines = NSStackView(views: [title, detail])
        lines.orientation = .vertical
        lines.alignment = .leading
        lines.spacing = 3
        addSubview(lines)
        lines.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            lines.leadingAnchor.constraint(equalTo: leadingAnchor, constant: DropLayout.inkWidth + 6),
            lines.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -14),
            lines.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        inkView.setAccessibilityElement(false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not built from a nib")
    }

    func show(_ text: DropText) {
        title.stringValue = text.title
        detail.stringValue = text.detail
        title.textColor = text.tone == .plain ? Palette.muted.nsColor : Palette.seal.nsColor
        layer?.borderColor = text.tone == .alert ? Palette.seal.nsColor.cgColor : Self.rule.cgColor
        layer?.borderWidth = text.tone == .alert ? 1.5 : 1
        setAccessibilityLabel("Inkwell: \(text.title), \(text.detail)")
    }
}
