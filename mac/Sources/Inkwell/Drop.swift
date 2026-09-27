// The Drop: the surface for everything live. A small paper panel near the bottom of the screen,
// with the ink on its left and two lines beside it, shown while something is live and hidden when
// idle. After a take that did not go in as it should (too short, no microphone, nothing selected
// for an edit...) it shows a note for a few seconds, the ink still (DictationModel.note). While
// nothing is live and the core offers to record a call, it asks (the consent Drop, with buttons).
// It never takes focus: while it shows, keystrokes and clicks elsewhere go where the user
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
    /// With buttons (the consent offer, the watchdog's "Allow system audio"): wider and taller.
    static let sizeWithActions = NSSize(width: 380, height: 112)
    static let inkWidth: CGFloat = 96
    static let cornerRadius: CGFloat = 16
    /// Above the bottom of the visible screen (the Dock's top when it shows).
    static let bottomMargin: CGFloat = 28
}

/// Shows and hides the Drop as the ink's state changes, with the core's offer to record a call
/// while nothing is live, and shows dictation's notes.
@MainActor
final class DropController {
    private let ink: ShellInk
    private let notes: DictationModel?
    private let panel = DropPanel()
    private let content = DropContentView()
    private(set) var isShown = false
    /// What the Drop's buttons do (the controller's meeting commands).
    var onAction: (DropText.Action) -> Void = { _ in }
    /// The note shown now (the ink still), until its time is up or something goes live.
    private(set) var noteShowing: DictationModel.Note?
    /// A note that came while something was live: shown when it ends.
    private var noteWaiting: DictationModel.Note?
    private var lastNoteSerial = 0
    private var wasLive = false

    /// How long a note stays up. One delayed call per note, not a timer: nothing ticks.
    static let noteDuration: Duration = .milliseconds(2_500)

    /// The state the Drop's ink shows.
    var inkState: InkState { content.inkView.state }
    /// Whether the Drop is the key window (never, by design).
    var panelIsKey: Bool { panel.isKeyWindow }
    /// What the Drop's lines say now.
    var shownText: DropText? { isShown ? content.text : nil }

    init(ink: ShellInk, notes: DictationModel? = nil) {
        self.ink = ink
        self.notes = notes
        panel.contentView = content
        content.onAction = { [weak self] action in self?.onAction(action) }
        lastNoteSerial = notes?.note?.serial ?? 0
        update()
        observe()
    }

    /// Brings the Drop in line with the ink's state, the core's offer and dictation's notes.
    /// What is live comes first; then a note (for its few seconds); then an offer.
    func update() {
        let state = ink.state
        takeNewNote(live: state.isLive)
        if state.isLive {
            noteShowing = nil
            wasLive = true
            display(ink.dropText, ink: state)
            return
        }
        if wasLive {
            wasLive = false
            if let waiting = noteWaiting {
                noteWaiting = nil
                showNote(waiting)
            }
        }
        if let note = noteShowing {
            display(note.text, ink: .idle)
        } else if ink.dropShows {
            // An offer to record a call, the ink still: nothing is recorded yet.
            display(ink.dropText, ink: state)
        } else if isShown {
            // Out first: the ink stops without drawing a last frame nobody would see.
            panel.orderOut(nil)
            isShown = false
            content.inkView.updateVisibility()
            content.inkView.state = .idle
        }
    }

    /// Shows `text` beside the ink in `state`, the panel sized for its buttons.
    private func display(_ text: DropText, ink state: InkState) {
        let size = text.actions.isEmpty ? DropLayout.size : DropLayout.sizeWithActions
        content.show(text)
        content.inkView.state = state
        if panel.frame.size != size {
            panel.setContentSize(size)
            if isShown { place() }
        }
        present()
    }

    /// A note dictation has not shown yet: now, or when what is live ends.
    private func takeNewNote(live: Bool) {
        guard let note = notes?.note, note.serial != lastNoteSerial else { return }
        lastNoteSerial = note.serial
        if live {
            noteWaiting = note
        } else {
            noteWaiting = nil
            showNote(note)
        }
    }

    private func showNote(_ note: DictationModel.Note) {
        noteShowing = note
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: Self.noteDuration)
            guard let self, self.noteShowing?.serial == note.serial else { return }
            self.noteShowing = nil
            self.update()
        }
    }

    private func present() {
        if !isShown {
            place()
            panel.orderFrontRegardless()
            isShown = true
            content.inkView.updateVisibility()
        }
    }

    /// Re-reads the state after every change the store or a held state makes: the change is
    /// handed to the next turn of the main queue, once the store has finished applying it.
    private func observe() {
        withObservationTracking {
            _ = ink.state
            _ = ink.dropText
            _ = ink.dropShows
            _ = notes?.note
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
        panel.setFrameOrigin(NSPoint(x: (visible.midX - panel.frame.width / 2).rounded(),
                                     y: visible.minY + DropLayout.bottomMargin))
    }
}

/// A button that takes the first click on a panel of an app that is not active (the Drop never
/// activates Inkwell, so every click on it is a first click).
final class DropButton: NSButton {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

/// The Drop's content: paper, the ink, two lines, and buttons when it offers something.
final class DropContentView: NSView {
    let inkView = InkView(frame: NSRect(x: 0, y: 0, width: DropLayout.inkWidth, height: DropLayout.size.height))
    private let title = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")
    private let buttons = NSStackView()
    private let inkHolder = NSView()
    /// What the lines say now.
    private(set) var text = DropText(title: "", detail: "")
    /// A button was clicked.
    var onAction: (DropText.Action) -> Void = { _ in }

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
        detail.maximumNumberOfLines = 2
        detail.cell?.wraps = true
        detail.preferredMaxLayoutWidth = DropLayout.sizeWithActions.width - DropLayout.inkWidth - 20
        buttons.orientation = .horizontal
        buttons.spacing = 8
        buttons.isHidden = true
        let lines = NSStackView(views: [title, detail, buttons])
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
        // The ink stays its size, centred on the panel's height, whichever size the panel takes.
        inkHolder.autoresizingMask = [.minYMargin, .maxYMargin]
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not built from a nib")
    }

    func show(_ text: DropText) {
        if text.actions != self.text.actions {
            buttons.arrangedSubviews.forEach { $0.removeFromSuperview() }
            for (index, action) in text.actions.enumerated() {
                let button = DropButton(title: action.title, target: self, action: #selector(clicked(_:)))
                button.tag = index
                button.bezelStyle = .push
                button.controlSize = .regular
                // The first is the offer's answer, in ink; the others are plain.
                if index == 0 {
                    button.bezelColor = Palette.ink.nsColor
                }
                buttons.addArrangedSubview(button)
            }
            buttons.isHidden = text.actions.isEmpty
        }
        self.text = text
        title.stringValue = text.title
        if text.liveWords {
            // The newest words matter: one line, the head cut, the last ones wet (the canvas).
            detail.maximumNumberOfLines = 1
            detail.cell?.wraps = false
            detail.lineBreakMode = .byTruncatingHead
            detail.attributedStringValue = Self.liveWords(text.detail)
        } else {
            detail.maximumNumberOfLines = 2
            detail.cell?.wraps = true
            detail.lineBreakMode = .byTruncatingTail
            detail.stringValue = text.detail
            detail.textColor = Palette.ink.nsColor
        }
        title.textColor = text.tone == .plain ? Palette.muted.nsColor : Palette.seal.nsColor
        layer?.borderColor = text.tone == .alert ? Palette.seal.nsColor.cgColor : Self.rule.cgColor
        layer?.borderWidth = text.tone == .alert ? 1.5 : 1
        // The live words are the user's: VoiceOver reads them (they are on screen), no log does.
        setAccessibilityLabel("Inkwell: \(text.title), \(text.detail)")
    }

    @objc private func clicked(_ sender: NSButton) {
        guard text.actions.indices.contains(sender.tag) else { return }
        onAction(text.actions[sender.tag])
    }

    /// Clicks a button as the user would (tests).
    func press(_ action: DropText.Action) {
        guard text.actions.contains(action) else { return }
        onAction(action)
    }

    /// Live words: dry in ink, the newest wet (italic, muted).
    static func liveWords(_ words: String) -> NSAttributedString {
        let font = NSFont.systemFont(ofSize: 14)
        let italic = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask)
        let wet = DropText.wetStart(words)
        let out = NSMutableAttributedString(
            string: String(words[..<wet]),
            attributes: [.font: font, .foregroundColor: Palette.ink.nsColor])
        out.append(NSAttributedString(
            string: String(words[wet...]),
            attributes: [.font: italic, .foregroundColor: Palette.muted.nsColor]))
        return out
    }
}
