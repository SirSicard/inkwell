// The menu-bar mark: the app icon, Halo rim, cut down to what reads at 18 pt: the plate's rim as a
// rounded square around the orb as a filled dot. It is a template, so macOS tints it for a light
// or dark menu bar, for wallpaper tinting and for an inactive display; the mark itself never takes
// a colour, and the live states draw over it (StatusItemController).
//
// Each scale is drawn on its own pixel grid rather than scaled from one drawing, so the rim's
// straight edges and the orb's centre fall on whole pixels at 1x and at 2x, crisp as a symbol is.
// The rim is filled as the space between two rounded squares, not stroked, so its edges are exact.
import AppKit
import QuartzCore

enum StatusGlyph {
    /// The mark's size, in points: a status item's image.
    static let size: CGFloat = 18

    /// The mark on one scale's grid, in pixels.
    struct Geometry: Equatable {
        /// From the image's edge to the rim's outer edge.
        let inset: Int
        /// The rim's width.
        let stroke: Int
        /// The orb's diameter.
        let orb: Int
        /// The rim's outer corner radius (the icon's plate: 185 of 824, about 22 %).
        let corner: CGFloat
    }

    /// 1x keeps a 2 px rim (a 1 px one reads as a hairline beside the menu bar's symbols); 2x
    /// draws 1.5 pt, a symbol's regular weight. The rim spans the same 14 pt at both.
    static func geometry(scale: Int) -> Geometry {
        scale >= 2
            ? Geometry(inset: 4, stroke: 3, orb: 12, corner: 6.5)
            : Geometry(inset: 2, stroke: 2, orb: 6, corner: 3)
    }

    /// The rim's outer edge, in points: where a state ring is laid over it.
    static let rimOuter = NSRect(x: 2, y: 2, width: 14, height: 14)
    /// The orb, in points: where a state colour is laid over it.
    static let orb = NSRect(x: 6, y: 6, width: 6, height: 6)

    /// The template image, with a bitmap for each scale.
    static func image() -> NSImage {
        let image = NSImage(size: NSSize(width: size, height: size))
        for scale in [1, 2] {
            if let rep = draw(scale: scale) { image.addRepresentation(rep) }
        }
        image.isTemplate = true
        image.accessibilityDescription = "Inkwell"
        return image
    }

    private static func draw(scale: Int) -> NSBitmapImageRep? {
        let side = Int(size) * scale
        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: side, pixelsHigh: side, bitsPerSample: 8, samplesPerPixel: 4,
            hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0),
            let context = NSGraphicsContext(bitmapImageRep: rep)
        else { return nil }
        let g = geometry(scale: scale)
        let cg = context.cgContext
        cg.clear(CGRect(x: 0, y: 0, width: side, height: side))
        cg.setFillColor(NSColor.black.cgColor)
        // In pixels: the rep is still sized in pixels while it is drawn (its points are set after).
        let outer = CGRect(x: g.inset, y: g.inset, width: side - 2 * g.inset, height: side - 2 * g.inset)
        let inner = outer.insetBy(dx: CGFloat(g.stroke), dy: CGFloat(g.stroke))
        let rim = CGMutablePath()
        rim.addRoundedRect(in: outer, cornerWidth: g.corner, cornerHeight: g.corner)
        let innerCorner = max(g.corner - CGFloat(g.stroke), 0.5)
        rim.addRoundedRect(in: inner, cornerWidth: innerCorner, cornerHeight: innerCorner)
        cg.addPath(rim)
        cg.fillPath(using: .evenOdd)
        let orbOrigin = (side - g.orb) / 2
        cg.fillEllipse(in: CGRect(x: orbOrigin, y: orbOrigin, width: g.orb, height: g.orb))
        context.flushGraphics()
        rep.size = NSSize(width: size, height: size)
        return rep
    }
}

/// The live state over the menu-bar mark. The mark stays a template, tinted by macOS; the state
/// is a coloured part laid exactly over one of its parts, rather than a coloured copy of the whole
/// glyph, which would lose the template's tinting for the menu bar, the wallpaper and an inactive
/// display:
///
///   glow, pulse   the orb in the state's colour (the pulse is a layer animation of this view's
///                 opacity, run by the render server: nothing redraws, the app never wakes)
///   ring          the rim filled clockwise from the top with the final pass, in their colour
///
/// It takes no clicks and is not an accessibility element: the button's label says the state.
final class StatusGlyphOverlay: NSView {
    /// The breath's animation, on the layer.
    static let breathKey = "inkwell.breath"

    private var shown = LiveIconFrame(look: .rest, colours: .unset, strength: 1)
    /// Someone can see the screen: the breath runs only then.
    private var awake = true
    /// How many times a frame asked for a redraw (the pulse's frames are opacity only).
    private(set) var redraws = 0

    override init(frame: NSRect) {
        super.init(frame: frame)
        // Layer-backed, so the pulse's opacity is composited, never redrawn.
        wantsLayer = true
        isHidden = true
        setAccessibilityElement(false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("not built from a nib")
    }

    /// Where the button draws the mark: centred in its bounds, on whole pixels.
    static func glyphRect(in bounds: NSRect) -> NSRect {
        NSRect(
            x: (bounds.width - StatusGlyph.size) / 2, y: (bounds.height - StatusGlyph.size) / 2,
            width: StatusGlyph.size, height: StatusGlyph.size)
    }

    /// It breathes by itself: LiveIcon shows it the pulse once and never ticks it.
    var breathesItself: Bool { true }

    func show(_ frame: LiveIconFrame) {
        // Only a new look or colour is drawn again; the strength is the animation's.
        var drawn = frame
        drawn.strength = 1
        if drawn != shown {
            shown = drawn
            redraws += 1
            needsDisplay = true
        }
        isHidden = frame.look == .rest
        breathe(frame.look.pulses && awake)
    }

    /// Asleep or locked, the breath comes off the layer, so the render server has nothing to run.
    func setAwake(_ awake: Bool) {
        self.awake = awake
        breathe(shown.look.pulses && awake)
    }

    /// Starts or stops the breath: the layer's opacity from full to `breathLow` and back over a
    /// breath, asking the render server for the pulse's low rate rather than the display's.
    private func breathe(_ on: Bool) {
        guard let layer else { return }
        let running = layer.animation(forKey: Self.breathKey) != nil
        if on && !running {
            let breath = CABasicAnimation(keyPath: "opacity")
            breath.fromValue = 1.0
            breath.toValue = LiveIcon.breathLow
            breath.duration = LiveIcon.breathPeriod / 2
            breath.autoreverses = true
            breath.repeatCount = .infinity
            breath.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            let fps = Float(LiveIcon.pulseFPS)
            breath.preferredFrameRateRange = CAFrameRateRange(minimum: fps - 1, maximum: fps + 1, preferred: fps)
            layer.add(breath, forKey: Self.breathKey)
        } else if !on && running {
            layer.removeAnimation(forKey: Self.breathKey)
        }
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        nil
    }

    override func draw(_ dirtyRect: NSRect) {
        let glyph = backingAlignedRect(Self.glyphRect(in: bounds), options: .alignAllEdgesNearest)
        let shownColours = shown.colours.shown
        switch shown.look {
        case .rest:
            break
        case .glow(let tone), .pulse(let tone):
            // Half a point over the template's orb all round, so no edge of it shows through.
            let orb = StatusGlyph.orb.offsetBy(dx: glyph.minX, dy: glyph.minY).insetBy(dx: -0.5, dy: -0.5)
            Self.colour(tone, shownColours).setFill()
            NSBezierPath(ovalIn: orb).fill()
        case .ring(let progress):
            // Exactly over the rim as this scale's bitmap draws it (2 pt at 1x, 1.5 pt at 2x).
            let scale = max(convertToBacking(NSSize(width: 1, height: 1)).width, 1)
            let g = StatusGlyph.geometry(scale: Int(scale.rounded()))
            let px = 1 / scale
            let stroke = CGFloat(g.stroke) * px
            let ring = glyph.insetBy(dx: CGFloat(g.inset) * px + stroke / 2, dy: CGFloat(g.inset) * px + stroke / 2)
            let corner = (g.corner - CGFloat(g.stroke) / 2) * px
            guard let cg = NSGraphicsContext.current?.cgContext else { return }
            let path = LiveIconArt.clockwiseFromTop(ring, corner: corner)
            let length = 2 * (ring.width + ring.height) - 8 * corner + 2 * .pi * corner
            cg.setLineWidth(stroke)
            cg.setLineCap(.butt)
            cg.setStrokeColor(GlowColours.nsColor(shownColours.them).cgColor)
            if let progress {
                let filled = length * CGFloat(min(max(progress, 0), 1))
                guard filled > 0 else { return }
                cg.setLineDash(phase: 0, lengths: [filled, length + 1])
            } else {
                cg.setLineDash(phase: 0, lengths: [length / 16, length / 16])
            }
            cg.addPath(path)
            cg.strokePath()
        }
    }

    /// The alert colour resolves against the menu bar's appearance as it draws, so it reads on a
    /// light or a dark menu bar; yours and theirs are the mode shown's, as every other dot.
    private static func colour(_ tone: LiveIconLook.Tone, _ pair: LiveIconColours.Pair) -> NSColor {
        switch tone {
        case .you: GlowColours.nsColor(pair.you)
        case .them: GlowColours.nsColor(pair.them)
        case .alert: Theme.dynamic { $0.alert.nsColor }
        }
    }
}
