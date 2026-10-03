// The orb on screen: a view backed by a transparent CAMetalLayer that draws the shared pipeline over
// whatever is behind it.
//
// It draws only while something is live (InkSchedule decides): the display link every live ink
// shares (InkClock) steps the simulation and draws one frame per vsync, reading the live levels at
// each tick. Idle, covered, or with motion stilled (Reduce Motion, or the user's "Always still"),
// it draws one still frame at most and then nothing. Every frame it presents is counted in
// InkRenderer.frames, which the shell budget (scripts/idle-budget.sh) reads.
//
// Given bounds (the main window's orb), it wanders (OrbWander): live, slowly, on the frames it
// draws anyway; at rest it glides to a new spot for a couple of seconds when it comes on screen,
// when the content behind it changes, and when its window becomes key again after a few minutes in
// one spot, then holds still again. No timer moves it: a window left alone at rest draws nothing.
// Hidden or covered it never moves; with motion stilled it takes each new spot in the one still
// frame, without a glide.
//
// The canvas's resolution: the backing scale, capped at 1.25 for a large canvas (over 180,000
// square points) and at 2 otherwise, so a window-sized orb is not drawn at full Retina resolution.
import AppKit
import Metal
import QuartzCore

/// A view that shows the ink. Main thread only.
@MainActor
public final class InkView: NSView {
    /// What the ink shows.
    public var state: InkState = .idle {
        didSet {
            guard state != oldValue else { return }
            simulation.state = state
            // Going to rest it stays where it got to (and that counts as its last move); going
            // live it sets off from there. From one live state to another it keeps going.
            if !(oldValue.isLive && state.isLive) {
                let now = CACurrentMediaTime()
                wander?.hold(at: now)
                if !state.isLive { lastMove = now }
            }
            perform(schedule.set(state: state))
        }
    }

    /// The region the orb's centre wanders in, as fractions of the view; nil keeps it at
    /// `placement` (the Drop, the first run). Its home is `placement`, where it starts.
    public var wanderBounds: OrbWander.Bounds? {
        didSet {
            guard wanderBounds != oldValue else { return }
            wander = wanderBounds.map {
                OrbWander(bounds: $0, start: SIMD2(placement.x, placement.yFromTop), random: wanderRandom)
            }
            perform(schedule.set(gliding: false))
            perform(schedule.invalidate())
        }
    }

    /// What the orb sits behind (the main window's screen). A change at rest, on screen, moves a
    /// wandering orb to a new spot.
    public var contentID: String? {
        didSet {
            guard contentID != oldValue else { return }
            moveAtRest()
        }
    }

    /// The orb's colours, as the theme resolves them.
    public var palette: OrbPalette = .neutral {
        didSet {
            guard palette != oldValue else { return }
            perform(schedule.invalidate())
        }
    }

    /// Where the orb sits in the view.
    public var placement: OrbPlacement = .centred {
        didSet {
            guard placement != oldValue else { return }
            perform(schedule.invalidate())
        }
    }

    /// How far blotting condenses the orb (InkSimulation.blotDepth).
    public var blotDepth = 1.0 {
        didSet {
            guard blotDepth != oldValue else { return }
            simulation.blotDepth = blotDepth
            perform(schedule.invalidate())
        }
    }

    /// Still: the orb holds one frame whatever is live, as under Reduce Motion (Settings >
    /// Appearance, "Always still").
    public var motionStill = false {
        didSet {
            guard motionStill != oldValue else { return }
            reduceMotionChanged()
        }
    }

    /// The live levels, read once per frame while live. Nil reads silence. It runs on the main
    /// thread inside the display link's callback, so it must return at once (ink_bands_read does:
    /// a copy, never a wait).
    public var levels: (@MainActor () -> InkLevels)?

    /// Frames this view has presented.
    public private(set) var framesDrawn = 0

    /// Whether the view is on the shared display link (live, on screen, motion allowed).
    public var isAnimating: Bool { clock.contains(self) }

    /// Why the orb cannot draw, if it cannot. The view then stays transparent.
    public private(set) var failure: InkRendererError?

    /// Whether the pipeline has arrived. Until then the view stays transparent and never runs a
    /// clock.
    public var isReady: Bool { pipeline != nil }

    /// For tests: treat the view as on screen without a window.
    var assumeOnScreen = false {
        didSet { visibilityChanged() }
    }

    /// For tests: pin Reduce Motion instead of reading the system setting. GitHub's macOS runners
    /// turn Reduce Motion on, so a test of live motion must not depend on the host's setting.
    var assumeReduceMotion: Bool? {
        didSet { reduceMotionChanged() }
    }

    /// For tests: the wander's random source, read when `wanderBounds` is set.
    var wanderRandom = InkRandom.system

    /// Its window becoming key again moves a resting orb only this long after its last move
    /// (tests shorten it).
    var restInterval = OrbWander.restInterval

    /// The orb's centre now, as fractions of the view.
    var orbCentre: SIMD2<Double> {
        wander?.position(at: CACurrentMediaTime()) ?? SIMD2(placement.x, placement.yFromTop)
    }

    /// The system's Reduce Motion setting, unless a test pinned it, or the user's "Always still".
    private var reduceMotion: Bool {
        motionStill || (assumeReduceMotion ?? NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
    }

    private var pipeline: InkPipeline?
    private var simulation = InkSimulation()
    private var wander: OrbWander?
    /// When the wandering orb last moved at rest.
    private var lastMove: CFTimeInterval = 0
    private var schedule = InkSchedule()
    private let clock: InkClock
    private var lastTimestamp: CFTimeInterval = 0
    private var firstTick = true
    private var canvas = (width: 0, height: 0)

    /// A view that draws with `loader`'s pipeline, driven by `clock` while live. It never waits
    /// for the compile: made before it finishes, the view stays transparent and starts drawing when
    /// the pipeline arrives.
    public init(frame: NSRect = .zero, loader: InkPipelineLoader = .shared, clock: InkClock = .shared) {
        self.clock = clock
        super.init(frame: frame)
        wantsLayer = true
        // The layer's content is the Metal drawable; AppKit never redraws it.
        layerContentsRedrawPolicy = .never
        // The prototype starts each ink at a random point in its slow motion.
        simulation.t = Double.random(in: 0..<30)
        _ = schedule.set(reduceMotion: reduceMotion)
        updateCanvas()
        if let outcome = loader.outcome {
            adopt(outcome)
        } else {
            loader.whenReady { [weak self] outcome in self?.adopt(outcome) }
        }
    }

    /// The compile finished: draw from now on, or stay transparent for good.
    private func adopt(_ outcome: InkPipelineLoader.Outcome) {
        switch outcome {
        case .success(let ready):
            pipeline = ready
            metalLayer?.device = ready.device
        case .failure(let error):
            failure = error
        }
        visibilityChanged()
    }

    @available(*, unavailable)
    public required init?(coder: NSCoder) {
        fatalError("not built from a nib")
    }

    public override func makeBackingLayer() -> CALayer {
        let layer = CAMetalLayer()
        layer.device = pipeline?.device
        layer.pixelFormat = InkPipeline.pixelFormat
        // The shader writes sRGB values directly, as the prototype's WebGL canvas does, and
        // premultiplied: the orb is composited over what is behind it.
        layer.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        layer.framebufferOnly = true
        layer.isOpaque = false
        layer.backgroundColor = nil
        return layer
    }

    private var metalLayer: CAMetalLayer? { layer as? CAMetalLayer }

    // MARK: Being on screen

    /// Whether the view listens for Reduce Motion changes: only while it is in a window, as it
    /// listens for its window being covered.
    private(set) var observesDisplayOptions = false

    public override func viewWillMove(toWindow newWindow: NSWindow?) {
        super.viewWillMove(toWindow: newWindow)
        if let window {
            NotificationCenter.default.removeObserver(
                self, name: NSWindow.didChangeOcclusionStateNotification, object: window)
            NotificationCenter.default.removeObserver(self, name: NSWindow.didBecomeKeyNotification, object: window)
        }
        if let newWindow {
            NotificationCenter.default.addObserver(
                self, selector: #selector(occlusionChanged(_:)),
                name: NSWindow.didChangeOcclusionStateNotification, object: newWindow)
            NotificationCenter.default.addObserver(
                self, selector: #selector(windowBecameKey(_:)), name: NSWindow.didBecomeKeyNotification,
                object: newWindow)
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
        // The setting may have changed while the view was out of a window, unheard.
        reduceMotionChanged()
        updateCanvas()
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
        updateCanvas()
        visibilityChanged()
    }

    public override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        updateCanvas()
    }

    /// On screen: in a visible window that is not fully covered, not hidden, and not empty.
    private var isOnScreen: Bool {
        if assumeOnScreen { return bounds.width >= 1 && bounds.height >= 1 }
        guard let window, window.isVisible, window.occlusionState.contains(.visible) else { return false }
        return !isHiddenOrHasHiddenAncestor && bounds.width >= 1 && bounds.height >= 1
    }

    /// Re-checks whether the view is on screen. Call it right after ordering its window in or
    /// out: the window server's occlusion notice comes a moment later, and until then a view in a
    /// window just ordered out would keep drawing.
    public func updateVisibility() {
        visibilityChanged()
    }

    /// The schedule sees a view that cannot draw yet (no pipeline) as not on screen: no clock, no
    /// still frame, until the pipeline arrives.
    private func visibilityChanged() {
        let onScreen = isOnScreen && pipeline != nil
        if onScreen && !schedule.onScreen && (!state.isLive || reduceMotion), var wander {
            // Coming on screen at rest (or still, live): a new spot, chosen before anything is
            // drawn, so a glide starts from where it was and, with motion stilled, the one still
            // frame is already there. Off screen the schedule's clock is stopped, so the action
            // ignored here is always .nothing; set(onScreen:) below acts on both.
            lastMove = CACurrentMediaTime()
            wander.move(at: lastMove, animated: !reduceMotion)
            self.wander = wander
            _ = reduceMotion ? schedule.invalidate() : schedule.set(gliding: true)
        }
        perform(schedule.set(onScreen: onScreen))
    }

    /// A wandering orb on screen goes to a new spot: at rest a glide, or at once with motion stilled
    /// (live too then: it has no frames of its own to wander on).
    private func moveAtRest() {
        guard var wander, !state.isLive || reduceMotion, schedule.onScreen else { return }
        let now = CACurrentMediaTime()
        wander.move(at: now, animated: !reduceMotion)
        self.wander = wander
        lastMove = now
        perform(reduceMotion ? schedule.invalidate() : schedule.set(gliding: true))
    }

    /// The user comes back to the window: a resting orb moves if it has held its spot for
    /// `restInterval`. No timer: a window left alone at rest draws nothing at all (the shell
    /// budget's idle phase, 0 frames in two minutes).
    private func becameKey() {
        guard CACurrentMediaTime() - lastMove >= restInterval else { return }
        moveAtRest()
    }

    /// Reduce Motion or "Always still" changed: a glide under way stops where it is.
    private func reduceMotionChanged() {
        if reduceMotion { wander?.hold(at: CACurrentMediaTime()) }
        perform(schedule.set(reduceMotion: reduceMotion))
    }

    // Both notifications arrive on the main thread; the hop covers a sender that ever does not.
    @objc private nonisolated func occlusionChanged(_ note: Notification) {
        onMain { $0.visibilityChanged() }
    }

    @objc private nonisolated func windowBecameKey(_ note: Notification) {
        onMain { $0.becameKey() }
    }

    @objc private nonisolated func displayOptionsChanged(_ note: Notification) {
        onMain { view in
            view.reduceMotionChanged()
        }
    }

    private nonisolated func onMain(_ body: @escaping @MainActor (InkView) -> Void) {
        if Thread.isMainThread {
            MainActor.assumeIsolated { body(self) }
        } else {
            DispatchQueue.main.async { MainActor.assumeIsolated { body(self) } }
        }
    }

    // MARK: Drawing

    /// The prototype's `_resize`: the canvas in pixels for the view's size and backing scale.
    private func updateCanvas() {
        let w = Double(bounds.width), h = Double(bounds.height)
        guard w >= 1, h >= 1 else { return }
        let backing = Double(window?.backingScaleFactor ?? NSScreen.main?.backingScaleFactor ?? 2)
        let scale = min(backing, w * h > 180_000 ? 1.25 : 2)
        let width = max(2, Int((w * scale).rounded())), height = max(2, Int((h * scale).rounded()))
        guard width != canvas.width || height != canvas.height else { return }
        canvas = (width, height)
        metalLayer?.drawableSize = CGSize(width: width, height: height)
        simulation.canvasWidth = Double(width)
        simulation.canvasHeight = Double(height)
        perform(schedule.invalidate())
    }

    private func perform(_ action: InkSchedule.Action) {
        switch action {
        case .nothing:
            break
        case .drawStill:
            drawStill()
        case .startClock:
            startClock()
        case .stopClock:
            stopClock()
        case .stopClockAndDrawStill:
            stopClock()
            drawStill()
        }
    }

    private func startClock() {
        guard !clock.contains(self) else { return }
        firstTick = true
        clock.add(self)
    }

    private func stopClock() {
        clock.remove(self)
    }

    /// One live frame, on the shared clock's tick: the prototype's `_frame`. dt is 1/60 s on the
    /// view's first tick, then the time since its last, clamped to 0...0.05 s.
    func clockTicked(at now: CFTimeInterval) {
        let dt = firstTick ? 0.016 : min(0.05, max(0, now - lastTimestamp))
        lastTimestamp = now
        firstTick = false
        guard state.isLive else {
            // A glide at rest: the settled frame, moving, until it arrives (then the still there).
            guard wander?.isMoving(at: now) == true else { return perform(schedule.set(gliding: false)) }
            return draw(motion: false, at: now)
        }
        let live = levels?() ?? .silent
        simulation.step(dt, snap: false, voice: .levels(near: live.near, far: live.far))
        wander?.wander(at: now)
        draw(motion: true, at: now)
    }

    /// The settled frame: weights at their targets, no voice, the shader's time stopped. A still
    /// frame shows the state at rest, not whatever level happened to be live.
    private func drawStill() {
        simulation.settle(voice: .silent)
        draw(motion: false, at: CACurrentMediaTime())
    }

    /// Where the orb sits at `now`: its placement, or where its wander has got to.
    private func placement(at now: CFTimeInterval) -> OrbPlacement {
        guard let wander else { return placement }
        let p = wander.position(at: now)
        return OrbPlacement(x: p.x, yFromTop: p.y, unit: placement.unit)
    }

    private func draw(motion: Bool, at now: CFTimeInterval) {
        guard let pipeline, let metalLayer, canvas.width > 0 else { return }
        guard let drawable = metalLayer.nextDrawable(),
            let commandBuffer = pipeline.queue.makeCommandBuffer()
        else { return }
        pipeline.encode(into: drawable.texture, commandBuffer: commandBuffer,
                        uniforms: simulation.uniforms(palette: palette, placement: placement(at: now), motion: motion))
        InkRenderer.gpuTimes.observe(commandBuffer)
        commandBuffer.present(drawable)
        commandBuffer.commit()
        framesDrawn += 1
        InkRenderer.frames.tick()
    }
}

extension InkView: InkClockClient {}
