// The ink on screen: a view backed by a CAMetalLayer that draws the shared pipeline.
//
// It draws only while something is live (InkSchedule decides): the display link every live ink
// shares (InkClock) steps the ink and draws one frame per vsync, reading the live levels at each
// tick. Idle, covered, or with Reduce
// Motion on, it draws one still frame at most and then nothing. Every frame it presents is counted
// in InkRenderer.frames, which the shell budget (scripts/idle-budget.sh) reads.
//
// The canvas follows the prototype's sizing rule: the backing scale, capped at 1.25 for a large
// canvas (over 180,000 square points) and at 2 otherwise, so a large ink zone is not drawn at full
// Retina resolution.
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
            perform(schedule.set(state: state))
        }
    }

    /// Whether the INKWELL wordmark is knocked out of the ink (Today's ink zone).
    public var showsWordmark = false {
        didSet {
            guard showsWordmark != oldValue else { return }
            markTexture = nil
            perform(schedule.invalidate())
        }
    }

    /// The wordmark's face.
    public var wordmarkFont: WordmarkFont = .system {
        didSet {
            guard wordmarkFont != oldValue else { return }
            markTexture = nil
            perform(schedule.invalidate())
        }
    }

    /// The ink body's centre height, 0...1 from the bottom.
    public var inkCentreHeight = 0.5 {
        didSet {
            guard inkCentreHeight != oldValue else { return }
            simulation.cy = inkCentreHeight
            perform(schedule.invalidate())
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

    /// Why the ink cannot draw, if it cannot. The view then shows plain paper.
    public private(set) var failure: InkRendererError?

    /// Whether the pipeline has arrived. Until then the view shows plain paper and never runs a
    /// clock.
    public var isReady: Bool { pipeline != nil }

    /// For tests: treat the view as on screen without a window.
    var assumeOnScreen = false {
        didSet { visibilityChanged() }
    }

    /// For tests: pin Reduce Motion instead of reading the system setting. GitHub's macOS runners
    /// turn Reduce Motion on, so a test of live motion must not depend on the host's setting.
    var assumeReduceMotion: Bool? {
        didSet { perform(schedule.set(reduceMotion: reduceMotion)) }
    }

    /// The system's Reduce Motion setting, unless a test pinned it.
    private var reduceMotion: Bool {
        assumeReduceMotion ?? NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    private var pipeline: InkPipeline?
    private var simulation = InkSimulation()
    private var schedule = InkSchedule()
    private let clock: InkClock
    private var lastTimestamp: CFTimeInterval = 0
    private var firstTick = true
    private var markTexture: (any MTLTexture)?
    private var canvas = (width: 0, height: 0)

    /// Paper, #F2EEE6: the layer's colour until its first frame, and where the ink cannot draw.
    private static let paper = CGColor(srgbRed: 0xF2 / 255, green: 0xEE / 255, blue: 0xE6 / 255, alpha: 1)

    /// A view that draws with `loader`'s pipeline, driven by `clock` while live. It never waits
    /// for the compile: made before it finishes, the view shows paper and starts drawing when the
    /// pipeline arrives.
    public init(frame: NSRect = .zero, loader: InkPipelineLoader = .shared, clock: InkClock = .shared) {
        self.clock = clock
        super.init(frame: frame)
        wantsLayer = true
        // The layer's content is the Metal drawable; AppKit never redraws it.
        layerContentsRedrawPolicy = .never
        // The prototype starts each ink at a random point in its slow motion.
        simulation.t = Double.random(in: 0..<30)
        simulation.cy = inkCentreHeight
        _ = schedule.set(reduceMotion: reduceMotion)
        updateCanvas()
        if let outcome = loader.outcome {
            adopt(outcome)
        } else {
            loader.whenReady { [weak self] outcome in self?.adopt(outcome) }
        }
    }

    /// The compile finished: draw from now on, or show paper for good.
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
        // The shader writes sRGB values directly, as the prototype's WebGL canvas does.
        layer.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        layer.framebufferOnly = true
        layer.isOpaque = true
        layer.backgroundColor = Self.paper
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
        // The setting may have changed while the view was out of a window, unheard.
        perform(schedule.set(reduceMotion: reduceMotion))
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
        perform(schedule.set(onScreen: isOnScreen && pipeline != nil))
    }

    // Both notifications arrive on the main thread; the hop covers a sender that ever does not.
    @objc private nonisolated func occlusionChanged(_ note: Notification) {
        onMain { $0.visibilityChanged() }
    }

    @objc private nonisolated func displayOptionsChanged(_ note: Notification) {
        onMain { view in
            view.perform(view.schedule.set(reduceMotion: view.reduceMotion))
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
        markTexture = nil
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
        let live = levels?() ?? .silent
        simulation.step(dt, snap: false, voice: .levels(near: live.near, far: live.far))
        draw()
    }

    /// The settled frame: droplets cleared, springs at their targets. No voice: a still frame
    /// shows the state at rest, not whatever level happened to be live.
    private func drawStill() {
        simulation.settle(voice: .silent)
        draw()
    }

    private func draw() {
        guard let pipeline, let metalLayer, canvas.width > 0 else { return }
        if showsWordmark, markTexture == nil {
            let mark = Wordmark.rasterize(width: canvas.width, height: canvas.height,
                                          pointWidth: Double(bounds.width), font: wordmarkFont)
            // A failed wordmark leaves the ink without it; the ink still draws.
            markTexture = try? pipeline.markTexture(mark)
        }
        guard let drawable = metalLayer.nextDrawable(),
            let commandBuffer = pipeline.queue.makeCommandBuffer()
        else { return }
        let mark = showsWordmark ? markTexture : nil
        pipeline.encode(into: drawable.texture, commandBuffer: commandBuffer,
                        uniforms: simulation.uniforms(hasMark: mark != nil), mark: mark)
        InkRenderer.gpuTimes.observe(commandBuffer)
        commandBuffer.present(drawable)
        commandBuffer.commit()
        framesDrawn += 1
        InkRenderer.frames.tick()
    }
}
