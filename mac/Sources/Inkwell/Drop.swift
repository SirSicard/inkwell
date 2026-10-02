// The Drop: the surface for everything live. A pill near the bottom of the screen, with a small
// orb on its left and two lines beside it, shown while something is live and hidden when idle. After a take that did not go in as it should (too short, no microphone, nothing selected
// for an edit...) it shows a note for a few seconds, the ink still (DictationModel.note). While
// nothing is live and the core offers to record a call, it asks (the consent Drop, with buttons).
// It never takes focus: while it shows, keystrokes and clicks elsewhere go where the user
// put them, and a click on the Drop itself does not activate Inkwell (mac/DROP-FOCUS-CHECKLIST.md
// is the check by hand).
//
// It follows the theme's mode: a night pill in dark mode, a day pill in light, with the orb in the
// theme's colours.
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
        // No appearance of its own: it follows the app's, which the theme's mode sets.
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// The Drop's measures, from the design canvas: a pill, its orb in a circle on the left.
enum DropLayout {
    static let size = NSSize(width: 440, height: 76)
    /// With buttons (the consent offer, the watchdog's "Allow system audio"): taller, its corners
    /// rounded rather than a pill's.
    static let sizeWithActions = NSSize(width: 440, height: 116)
    /// The orb's circle, and its inset from the pill's left edge.
    static let orbSize: CGFloat = 58
    static let orbInset: CGFloat = 10
    /// Where the lines start.
    static let inkWidth: CGFloat = orbInset + orbSize + 14
    static let cornerRadius: CGFloat = 38
    static let cornerRadiusWithActions: CGFloat = 28
    /// Above the bottom of the visible screen (the Dock's top when it shows).
    static let bottomMargin: CGFloat = 28
}

/// Shows and hides the Drop as the ink's state changes, with the core's offer to record a call
/// while nothing is live, and shows dictation's notes.
@MainActor
final class DropController {
    private let ink: ShellInk
    private let notes: DictationModel?
    /// The orb's colours and whether it moves.
    private let theme: GlowTheme?
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

    init(ink: ShellInk, notes: DictationModel? = nil, theme: GlowTheme? = nil) {
        self.ink = ink
        self.notes = notes
        self.theme = theme
        panel.contentView = content
        content.onAction = { [weak self] action in self?.onAction(action) }
        lastNoteSerial = notes?.note?.serial ?? 0
        update()
        observe()
    }

    /// Brings the Drop in line with the ink's state, the core's offer and dictation's notes.
    /// What is live comes first; then a note (for its few seconds); then an offer.
    func update() {
        if let theme {
            content.inkView.palette = theme.palette
            content.inkView.motionStill = theme.motionStill
        }
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
        content.setCorner(text.actions.isEmpty ? DropLayout.cornerRadius : DropLayout.cornerRadiusWithActions)
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
            _ = theme?.palette
            _ = theme?.motionStill
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

/// The Drop's content: the pill, the orb in its circle, two lines, and buttons when it offers
/// something. Its colours are the mode's tokens, re-read whenever the appearance changes.
final class DropContentView: NSView {
    let inkView = InkView(frame: NSRect(x: 0, y: 0, width: DropLayout.orbSize, height: DropLayout.orbSize))
    private let title = NSTextField(labelWithString: "")
    private let detail = NSTextField(labelWithString: "")
    private let buttons = NSStackView()
    private let orbHolder = NSView()
    /// What the lines say now.
    private(set) var text = DropText(title: "", detail: "")
    /// A button was clicked.
    var onAction: (DropText.Action) -> Void = { _ in }

    /// The pill and its rule, by mode.
    private static let fill = Theme.dynamic { $0.card.nsColor }
    private static let rule = Theme.dynamic { $0.border.nsColor }
    private static let textColor = Theme.dynamic { $0.text.nsColor }
    private static let secondary = Theme.dynamic { $0.secondary.nsColor }
    private static let alert = Theme.dynamic { $0.alert.nsColor }
    /// The detail's face: New York.
    private static let lineFont = NSFont(
        descriptor: NSFont.systemFont(ofSize: 17).fontDescriptor.withDesign(.serif) ?? NSFont.systemFont(ofSize: 17).fontDescriptor,
        size: 17) ?? NSFont.systemFont(ofSize: 17)

    init() {
        super.init(frame: NSRect(origin: .zero, size: DropLayout.size))
        wantsLayer = true
        guard let layer else { return }
        layer.cornerRadius = DropLayout.cornerRadius
        layer.cornerCurve = .continuous
        layer.masksToBounds = true
        layer.borderWidth = 1

        // The orb in its circle, centred on the panel's height whichever size the panel takes.
        orbHolder.frame = NSRect(
            x: DropLayout.orbInset, y: (DropLayout.size.height - DropLayout.orbSize) / 2,
            width: DropLayout.orbSize, height: DropLayout.orbSize)
        orbHolder.wantsLayer = true
        orbHolder.layer?.cornerRadius = DropLayout.orbSize / 2
        orbHolder.layer?.masksToBounds = true
        inkView.placement = Glow.Orb.drop
        orbHolder.addSubview(inkView)
        orbHolder.autoresizingMask = [.minYMargin, .maxYMargin]
        addSubview(orbHolder)

        title.font = .systemFont(ofSize: 12, weight: .semibold)
        detail.font = Self.lineFont
        detail.lineBreakMode = .byTruncatingTail
        detail.maximumNumberOfLines = 2
        detail.cell?.wraps = true
        detail.preferredMaxLayoutWidth = DropLayout.sizeWithActions.width - DropLayout.inkWidth - 24
        buttons.orientation = .horizontal
        buttons.spacing = 8
        buttons.isHidden = true
        let lines = NSStackView(views: [title, detail, buttons])
        lines.orientation = .vertical
        lines.alignment = .leading
        lines.spacing = 2
        addSubview(lines)
        lines.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            lines.leadingAnchor.constraint(equalTo: leadingAnchor, constant: DropLayout.inkWidth),
            lines.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -24),
            lines.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        inkView.setAccessibilityElement(false)
        applyColours()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not built from a nib")
    }

    /// A pill alone, rounded corners with buttons.
    func setCorner(_ radius: CGFloat) {
        layer?.cornerRadius = radius
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        applyColours()
    }

    /// The layer's colours are CGColors, fixed when set: re-read in the appearance shown.
    private func applyColours() {
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = Self.fill.cgColor
            layer?.borderColor = (text.tone == .alert ? Self.alert : Self.rule).cgColor
        }
        title.textColor = text.tone == .plain ? Self.secondary : Self.alert
        if text.liveWords {
            detail.attributedStringValue = Self.liveWords(text.detail)
        } else {
            detail.textColor = Self.textColor
        }
    }

    func show(_ text: DropText) {
        if text.actions != self.text.actions {
            buttons.arrangedSubviews.forEach { $0.removeFromSuperview() }
            for (index, action) in text.actions.enumerated() {
                let button = DropButton(title: action.title, target: self, action: #selector(clicked(_:)))
                button.tag = index
                button.bezelStyle = .push
                button.controlSize = .regular
                // The first is the offer's answer, prominent; the others are plain.
                if index == 0 {
                    button.keyEquivalent = ""
                    button.bezelColor = Self.textColor
                    button.contentTintColor = Theme.dynamic { $0.buttonLabel.nsColor }
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
        } else {
            detail.maximumNumberOfLines = 2
            detail.cell?.wraps = true
            detail.lineBreakMode = .byTruncatingTail
            detail.stringValue = text.detail
        }
        layer?.borderWidth = text.tone == .alert ? 1.5 : 1
        applyColours()
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

    /// Live words: dry in the text colour, the newest wet (italic, secondary).
    static func liveWords(_ words: String) -> NSAttributedString {
        let font = lineFont
        let italic = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask)
        let wet = DropText.wetStart(words)
        let out = NSMutableAttributedString(
            string: String(words[..<wet]),
            attributes: [.font: font, .foregroundColor: textColor])
        out.append(NSAttributedString(
            string: String(words[wet...]),
            attributes: [.font: italic, .foregroundColor: secondary]))
        return out
    }
}
