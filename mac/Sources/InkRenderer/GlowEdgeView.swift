// The window's edge glow: native gradient strokes round the edge of the window, above the content,
// taking no clicks. Yours while you dictate; yours and theirs in a call, leaning toward whoever is
// speaking; gone once the call is blotted (EdgeGlowFrame has the rule).
//
// The glow is a stack of strokes, each wider and fainter, all in the same horizontal gradient: one
// CAGradientLayer per stroke, masked by that stroke's shape and faded to its opacity, so the window
// server composites it and nothing is drawn on the CPU. It moves only while something is live, on
// the display link every live ink shares (InkClock); idle, covered, switched off, or with motion
// stilled, it shows one still frame at most and then nothing (InkSchedule).
import AppKit
import QuartzCore

/// The edge glow over a window's content. Main thread only.
@MainActor
public final class GlowEdgeView: NSView {
    /// What is live.
    public var state: InkState = .idle {
        didSet {
            guard state != oldValue else { return }
            simulation.state = state
            perform(schedule.set(state: state))
        }
    }

    /// The colours (yours and theirs).
    public var palette: OrbPalette = .neutral {
        didSet {
            guard palette != oldValue else { return }
            perform(schedule.invalidate())
        }
    }

    /// The strokes, the blend and the window's corner.
    public var style: EdgeGlowStyle {
        didSet {
            guard style != oldValue else { return }
            rebuildStrokes()
            perform(schedule.invalidate())
        }
    }

    /// Off: nothing shows and nothing runs (Settings > Appearance).
    public var isOn = true {
        didSet {
            guard isOn != oldValue else { return }
            strokeHolder.isHidden = !isOn
            visibilityChanged()
        }
    }

    /// Still: the glow holds one frame, as under Reduce Motion (Settings > Appearance).
    public var motionStill = false {
        didSet {
            guard motionStill != oldValue else { return }
            perform(schedule.set(reduceMotion: reduceMotion))
        }
    }

    /// The live levels, read once per frame while live (as the orb's are). Nil reads silence.
    public var levels: (@MainActor () -> InkLevels)?

    /// Frames this view has drawn.
    public private(set) var framesDrawn = 0

    /// Whether it is on the shared display link.
    public var isAnimating: Bool { clock.contains(self) }

    /// The frame shown now; nil when nothing shows.
    public private(set) var shown: EdgeGlowFrame?

    /// For tests: treat the view as on screen without a window.
    var assumeOnScreen = false {
        didSet { visibilityChanged() }
    }

    /// For tests: pin Reduce Motion instead of reading the system setting.
    var assumeReduceMotion: Bool? {
        didSet { perform(schedule.set(reduceMotion: reduceMotion)) }
    }

    private var reduceMotion: Bool {
        motionStill || (assumeReduceMotion ?? NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
    }

    private var simulation = InkSimulation()
    private var schedule = InkSchedule()
    private let clock: InkClock
    private var lastTimestamp: CFTimeInterval = 0
    private var firstTick = true
    private let strokeHolder = CALayer()
    private var strokes: [(gradient: CAGradientLayer, mask: CAShapeLayer)] = []
    private var observesDisplayOptions = false

    public init(frame: NSRect = .zero, style: EdgeGlowStyle, clock: InkClock = .shared) {
        self.style = style
        self.clock = clock
        super.init(frame: frame)
        wantsLayer = true
        layerContentsRedrawPolicy = .never
        layer?.addSublayer(strokeHolder)
        strokeHolder.isHidden = true
        _ = schedule.set(reduceMotion: reduceMotion)
        rebuildStrokes()
        setAccessibilityElement(false)
    }

    @available(*, unavailable)
    public required init?(coder: NSCoder) {
        fatalError("not built from a nib")
    }

    /// It takes no clicks: everything under it stays usable.
    public override func hitTest(_ point: NSPoint) -> NSView? { nil }

    // MARK: Being on screen

    public override func viewWillMove(toWindow newWindow: NSWindow?) {
        super.viewWillMove(toWindow: newWindow)
        if let window {
            NotificationCenter.default.removeObserver(
                self, name: NSWindow.didChangeOcclusionStateNotification, object: window)
        }
        if let newWindow {
            NotificationCenter.default.addObserver(
                self, selector: #selector(occlusionChanged(_:)),
                name: NSWindow.didChangeOcclusionStateNotification, object: newWindow)
        }
        let workspace = NSWorkspace.shared.notificationCenter
        if newWindow != nil, !observesDisplayOptions {
            workspace.addObserver(
                self, selector: #selector(displayOptionsChanged(_:)),
                name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
            observesDisplayOptions = true
        } else if newWindow == nil, observesDisplayOptions {
            workspace.removeObserver(
                self, name: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil)
            observesDisplayOptions = false
        }
    }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        perform(schedule.set(reduceMotion: reduceMotion))
        layoutStrokes()
        visibilityChanged()
    }

    public override func viewDidHide() {
        super.viewDidHide()
        visibilityChanged()
    }

    public override func viewDidUnhide() {
        super.viewDidUnhide()
        visibilityChanged()
    }

    public override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        layoutStrokes()
        visibilityChanged()
        perform(schedule.invalidate())
    }

    public override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        layoutStrokes()
    }

    private var isOnScreen: Bool {
        if assumeOnScreen { return bounds.width >= 1 && bounds.height >= 1 }
        guard let window, window.isVisible, window.occlusionState.contains(.visible) else { return false }
        return !isHiddenOrHasHiddenAncestor && bounds.width >= 1 && bounds.height >= 1
    }

    /// Switched off counts as not on screen: no clock and no frame.
    private func visibilityChanged() {
        perform(schedule.set(onScreen: isOnScreen && isOn))
    }

    @objc private nonisolated func occlusionChanged(_ note: Notification) {
        onMain { $0.visibilityChanged() }
    }

    @objc private nonisolated func displayOptionsChanged(_ note: Notification) {
        onMain { view in
            view.perform(view.schedule.set(reduceMotion: view.reduceMotion))
        }
    }

    private nonisolated func onMain(_ body: @escaping @MainActor (GlowEdgeView) -> Void) {
        if Thread.isMainThread {
            MainActor.assumeIsolated { body(self) }
        } else {
            DispatchQueue.main.async { MainActor.assumeIsolated { body(self) } }
        }
    }

    // MARK: Drawing

    private func rebuildStrokes() {
        strokes.forEach { $0.gradient.removeFromSuperlayer() }
        strokes = style.strokes.map { stroke in
            let gradient = CAGradientLayer()
            gradient.startPoint = CGPoint(x: 0, y: 0.5)
            gradient.endPoint = CGPoint(x: 1, y: 0.5)
            gradient.opacity = Float(stroke.alpha)
            let mask = CAShapeLayer()
            mask.fillColor = nil
            mask.strokeColor = CGColor(gray: 0, alpha: 1)
            mask.lineWidth = stroke.width
            gradient.mask = mask
            strokeHolder.addSublayer(gradient)
            return (gradient, mask)
        }
        layoutStrokes()
    }

    /// Every stroke follows the window's edge: a rounded rectangle inset by a point, each stroke
    /// centred on it (its outer half falls outside the window and is cut).
    private func layoutStrokes() {
        let scale = window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        strokeHolder.frame = bounds
        let edge = bounds.insetBy(dx: 1, dy: 1)
        let radius = min(style.cornerRadius, edge.width / 2, edge.height / 2)
        let path = edge.width > 0 && edge.height > 0
            ? CGPath(roundedRect: edge, cornerWidth: max(0, radius), cornerHeight: max(0, radius), transform: nil)
            : CGPath(rect: .zero, transform: nil)
        for stroke in strokes {
            stroke.gradient.frame = bounds
            stroke.gradient.contentsScale = scale
            stroke.mask.frame = bounds
            stroke.mask.contentsScale = scale
            stroke.mask.path = path
        }
        CATransaction.commit()
    }

    private func perform(_ action: InkSchedule.Action) {
        switch action {
        case .nothing:
            break
        case .drawStill:
            drawStill()
        case .startClock:
            if !clock.contains(self) {
                firstTick = true
                clock.add(self)
            }
        case .stopClock:
            clock.remove(self)
        case .stopClockAndDrawStill:
            clock.remove(self)
            drawStill()
        }
    }

    /// One live frame, on the shared clock's tick.
    func clockTicked(at now: CFTimeInterval) {
        let dt = firstTick ? 0.016 : min(0.05, max(0, now - lastTimestamp))
        lastTimestamp = now
        firstTick = false
        let live = levels?() ?? .silent
        simulation.step(dt, snap: false, voice: .levels(near: live.near, far: live.far))
        draw()
    }

    /// The settled frame: weights at their targets, no voice.
    private func drawStill() {
        simulation.settle(voice: .silent)
        draw()
    }

    private func draw() {
        let frame = EdgeGlowFrame.compute(
            w: simulation.w, you: simulation.envA, them: simulation.envB, t: simulation.t,
            palette: palette, style: style)
        shown = frame
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if let frame, isOn {
            let colors = frame.stops.map {
                CGColor(srgbRed: $0.color.x, green: $0.color.y, blue: $0.color.z, alpha: $0.color.w)
            }
            let locations = frame.stops.map { NSNumber(value: $0.location) }
            for stroke in strokes {
                stroke.gradient.colors = colors
                stroke.gradient.locations = locations
            }
            strokeHolder.isHidden = false
        } else {
            strokeHolder.isHidden = true
        }
        CATransaction.commit()
        framesDrawn += 1
    }
}

extension GlowEdgeView: InkClockClient {}
