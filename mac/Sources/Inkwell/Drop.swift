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
    /// rounded rather than a pill's. At least this; taller when its lines and buttons need it
    /// (the offer's four buttons take two rows).
    static let sizeWithActions = NSSize(width: 440, height: 116)
    /// Above and below the lines and buttons, with buttons.
    static let actionsPadding: CGFloat = 16
    /// The room the lines and buttons have: the pill less the orb's side and the right margin.
    static var linesWidth: CGFloat { sizeWithActions.width - inkWidth - 24 }
    /// Between buttons, and between their rows.
    static let buttonSpacing: CGFloat = 8
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
    /// A note with a button (no speech model: Today, for the download) stays long enough to be pressed.
    static let noteWithActionsDuration: Duration = .seconds(8)

    /// The state the Drop's ink shows.
    var inkState: InkState { content.inkView.state }
    /// Whether the Drop is the key window (never, by design).
    var panelIsKey: Bool { panel.isKeyWindow }
    /// What the Drop's lines say now.
    var shownText: DropText? { isShown ? content.text : nil }
    /// The Drop's content (tests and renders).
    var contentView: DropContentView { content }

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
            // The theme's palette as it is: at rest (a note, an offer) the Drop's orb leans
            // toward the preset too. It sits beside the text, not under it.
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
        content.show(text)
        let size = content.fittingPanelSize
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
        if note.text.yields, noteShowing != nil || noteWaiting != nil { return }
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
            try? await Task.sleep(for: note.text.actions.isEmpty ? Self.noteDuration : Self.noteWithActionsDuration)
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
    private let recordingHeader = NSView()
    private let recordingApp = NSTextField(labelWithString: "")
    private let recordingBadge = NSTextField(labelWithString: "")
    /// The buttons, in rows: as many to a row as fit beside the orb.
    private let buttons = NSStackView()
    private let orbHolder = NSView()
    private let lines: NSStackView
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
        lines = NSStackView(views: [title, recordingHeader, detail, buttons])
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

        recordingHeader.isHidden = true
        recordingApp.font = .systemFont(ofSize: 19, weight: .semibold)
        recordingApp.alignment = .center
        recordingApp.lineBreakMode = .byTruncatingMiddle
        recordingApp.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        recordingBadge.font = .systemFont(ofSize: 10, weight: .bold)
        recordingBadge.alignment = .center
        recordingBadge.wantsLayer = true
        recordingBadge.layer?.cornerRadius = 8
        recordingBadge.layer?.masksToBounds = true
        let badge = NSMutableAttributedString(string: "● ", attributes: [.foregroundColor: NSColor(srgbRed: 1, green: 0.45, blue: 0.36, alpha: 1)])
        badge.append(NSAttributedString(string: "REC", attributes: [.foregroundColor: NSColor.white]))
        recordingBadge.attributedStringValue = badge
        for field in [recordingApp, recordingBadge] {
            recordingHeader.addSubview(field)
            field.translatesAutoresizingMaskIntoConstraints = false
        }
        NSLayoutConstraint.activate([
            recordingHeader.widthAnchor.constraint(equalToConstant: DropLayout.linesWidth),
            recordingHeader.heightAnchor.constraint(equalToConstant: 25),
            recordingBadge.trailingAnchor.constraint(equalTo: recordingHeader.trailingAnchor),
            recordingBadge.centerYAnchor.constraint(equalTo: recordingHeader.centerYAnchor),
            recordingBadge.widthAnchor.constraint(equalToConstant: 52),
            recordingBadge.heightAnchor.constraint(equalToConstant: 20),
            recordingApp.leadingAnchor.constraint(equalTo: recordingHeader.leadingAnchor),
            recordingApp.trailingAnchor.constraint(equalTo: recordingBadge.leadingAnchor, constant: -8),
            recordingApp.centerYAnchor.constraint(equalTo: recordingHeader.centerYAnchor),
        ])
        title.font = .systemFont(ofSize: 12, weight: .semibold)
        detail.font = Self.lineFont
        // Wrapped by word, the last line cut: a tail-truncating field draws one line only, which
        // cut the reminder to tell the others off the consent offer.
        detail.lineBreakMode = .byWordWrapping
        detail.cell?.truncatesLastVisibleLine = true
        detail.maximumNumberOfLines = 2
        detail.cell?.wraps = true
        detail.preferredMaxLayoutWidth = DropLayout.linesWidth
        buttons.orientation = .vertical
        buttons.alignment = .leading
        buttons.spacing = DropLayout.buttonSpacing
        buttons.isHidden = true
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
            let colour = inkView.palette.yA
            let wash = NSColor(srgbRed: CGFloat(colour.x), green: CGFloat(colour.y), blue: CGFloat(colour.z), alpha: 1)
            // Recording alone takes a soft wash from the active orb palette; offers keep their surface.
            let fill = text.tone == .recording ? Self.fill.blended(withFraction: 0.1, of: wash) ?? Self.fill : Self.fill
            layer?.backgroundColor = fill.cgColor
            // A palette surface remains behind the badge in Light and Dark, including Metal fallback.
            recordingBadge.layer?.backgroundColor = Glow.night.background.nsColor.cgColor
            orbHolder.layer?.backgroundColor = NSColor(
                srgbRed: CGFloat(colour.x), green: CGFloat(colour.y), blue: CGFloat(colour.z), alpha: 0.18).cgColor
            layer?.borderColor = (text.tone == .alert ? Self.alert : Self.rule).cgColor
        }
        recordingApp.textColor = Self.textColor
        title.textColor = text.tone == .plain ? Self.secondary : Self.alert
        if let primary = shownButtons.first {
            primary.attributedTitle = NSAttributedString(string: primary.title, attributes: [
                .foregroundColor: Theme.dynamic { $0.buttonLabel.nsColor },
                .font: primary.font ?? NSFont.systemFont(ofSize: NSFont.systemFontSize),
            ])
        }
        if text.liveWords {
            detail.attributedStringValue = Self.liveWords(text.detail)
        } else {
            detail.textColor = Self.textColor
        }
    }

    /// The panel's size for what it shows: the pill alone, or tall enough for the lines and the
    /// rows of buttons.
    var fittingPanelSize: NSSize {
        guard !text.actions.isEmpty else { return DropLayout.size }
        layoutSubtreeIfNeeded()
        let height = ceil(lines.fittingSize.height) + 2 * DropLayout.actionsPadding
        return NSSize(width: DropLayout.sizeWithActions.width, height: max(DropLayout.sizeWithActions.height, height))
    }

    /// How many lines the detail takes at the Drop's width, and how many it may (tests: nothing
    /// said is cut).
    var detailLines: (needed: Int, allowed: Int) {
        let font = detail.font ?? Self.lineFont
        let height = (detail.stringValue as NSString).boundingRect(
            with: NSSize(width: DropLayout.linesWidth, height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading], attributes: [.font: font]
        ).height
        let line = NSLayoutManager().defaultLineHeight(for: font)
        return (Int((height / line).rounded()), detail.maximumNumberOfLines)
    }

    /// The buttons now, in order (tests).
    var shownButtons: [NSButton] {
        buttons.arrangedSubviews.flatMap { ($0 as? NSStackView)?.arrangedSubviews ?? [] }.compactMap { $0 as? NSButton }
    }

    func show(_ text: DropText) {
        if text.actions != self.text.actions {
            buttons.arrangedSubviews.forEach { $0.removeFromSuperview() }
            var row: NSStackView?
            var rowWidth: CGFloat = 0
            for (index, action) in text.actions.enumerated() {
                let button = DropButton(title: action.title, target: self, action: #selector(clicked(_:)))
                button.tag = index
                button.bezelStyle = .push
                button.controlSize = .regular
                // A long app name is cut in its middle, never past the Drop's edge.
                button.lineBreakMode = .byTruncatingMiddle
                button.setAccessibilityHelp(action.hint)
                // The first is the offer's answer, prominent; the others are plain.
                if index == 0 {
                    button.keyEquivalent = ""
                    button.bezelColor = Self.textColor
                    // AppKit does not consistently apply contentTintColor to a push button title.
                    // Its attributed title is resolved with the palette below.
                }
                let width = min(button.fittingSize.width, DropLayout.linesWidth)
                button.widthAnchor.constraint(lessThanOrEqualToConstant: DropLayout.linesWidth).isActive = true
                if let current = row, rowWidth + DropLayout.buttonSpacing + width <= DropLayout.linesWidth {
                    current.addArrangedSubview(button)
                    rowWidth += DropLayout.buttonSpacing + width
                } else {
                    let next = NSStackView(views: [button])
                    next.orientation = .horizontal
                    next.spacing = DropLayout.buttonSpacing
                    buttons.addArrangedSubview(next)
                    row = next
                    rowWidth = width
                }
            }
            buttons.isHidden = text.actions.isEmpty
        }
        self.text = text
        title.stringValue = text.title
        let recording = text.tone == .recording && text.recordingName != nil
        recordingHeader.isHidden = !recording
        title.isHidden = recording && text.actions.isEmpty
        recordingApp.stringValue = text.recordingName ?? ""
        if text.liveWords {
            // The newest words matter: one line, the head cut, the last ones wet (the canvas).
            detail.maximumNumberOfLines = 1
            detail.cell?.wraps = false
            detail.lineBreakMode = .byTruncatingHead
        } else {
            // With buttons, room for a third line (an Always app asked instead says why).
            detail.maximumNumberOfLines = text.actions.isEmpty ? 2 : 3
            detail.cell?.wraps = true
            detail.lineBreakMode = .byWordWrapping
            detail.cell?.truncatesLastVisibleLine = true
            detail.stringValue = text.detail
        }
        layer?.borderWidth = text.tone == .alert ? 1.5 : 1
        applyColours()
        // The live words are the user's: VoiceOver reads them (they are on screen), no log does.
        setAccessibilityLabel("Inkwell: \(text.title), \(text.detail)")
    }

    /// How many lines the detail is laid out on now (tests).
    var detailShownLines: Int {
        layoutSubtreeIfNeeded()
        let line = NSLayoutManager().defaultLineHeight(for: detail.font ?? Self.lineFont)
        return Int((detail.frame.height / line).rounded())
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
