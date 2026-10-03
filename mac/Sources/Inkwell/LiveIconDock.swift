// The live icon in the Dock. The tile exists only while the main window is open (the app is a
// menu-bar app otherwise), so the app attaches this surface when the window opens and detaches it
// when it closes.
//
// At rest the tile has no view of its own and macOS draws the bundle's icon: exactly the static
// one, with nothing redrawn. Live, a view draws the same icon (the app's icon image, read once)
// with the frame over it, and the tile is redrawn once per frame: once per change for a still
// look, at the pulse's 7 fps while recording.
import AppKit

/// The Dock tile, as the live icon uses it (a protocol, so tests count its redraws).
@MainActor
protocol LiveIconTile: AnyObject {
    var contentView: NSView? { get set }
    var size: NSSize { get }
    func display()
}

extension NSDockTile: LiveIconTile {}

@MainActor
final class LiveIconDock: LiveIconSurface {
    private let tile: LiveIconTile
    private let view: View

    /// `base`: the app icon, as the bundle carries it.
    init(tile: LiveIconTile, base: NSImage) {
        self.tile = tile
        view = View(base: base)
    }

    func show(_ frame: LiveIconFrame) {
        guard frame.look != .rest else {
            // Back to the bundle's icon, drawn by macOS.
            guard tile.contentView != nil else { return }
            tile.contentView = nil
            tile.display()
            return
        }
        view.shown = frame
        if tile.contentView !== view {
            view.frame = NSRect(origin: .zero, size: tile.size)
            tile.contentView = view
        }
        tile.display()
    }

    /// Draws the icon with the frame over it.
    private final class View: NSView {
        let base: NSImage
        var shown = LiveIconFrame(look: .rest, colours: .unset, strength: 1)

        init(base: NSImage) {
            self.base = base
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("not built from a nib")
        }

        override func draw(_ dirtyRect: NSRect) {
            LiveIconArt.draw(shown, base: base, in: bounds)
        }
    }
}

/// The live looks drawn over the app icon, in the icon's own geometry (design/icon/make_icon.py:
/// Apple's grid, the plate 824 of 1024 inset 100 with corners of 185, the orb's centre 51 % down
/// the plate). The plate is night in either mode, so the colours are night's.
enum LiveIconArt {
    /// The plate's inset and corner radius, as shares of the icon's side.
    static let plateInset: CGFloat = 100 / 1024
    static let plateCorner: CGFloat = 185 / 1024
    /// The orb's centre, as shares of the icon's side, from the bottom left.
    static let orbCentre = CGPoint(x: 0.5, y: 1 - (100 + 8.24 * 51) / 1024)
    /// How far the orb's glow reaches: a little past the icon's own orb and its blur.
    static let glowRadius: CGFloat = 0.2
    /// The ring's width, and its centre line's inset from the plate's edge, as shares of the side:
    /// it lies over the plate's glowing edge.
    static let ringWidth: CGFloat = 0.028
    static let ringInset: CGFloat = 0.026
    /// The alert colour on night.
    static let alert = GlowColours.rgb(Glow.night.alert)

    static func plateRect(in rect: NSRect) -> NSRect {
        rect.insetBy(dx: rect.width * plateInset, dy: rect.height * plateInset)
    }

    /// The ring's centre line.
    static func ringRect(in rect: NSRect) -> NSRect {
        plateRect(in: rect).insetBy(dx: rect.width * ringInset, dy: rect.height * ringInset)
    }

    static func draw(_ frame: LiveIconFrame, base: NSImage, in rect: NSRect) {
        base.draw(in: rect)
        guard let cg = NSGraphicsContext.current?.cgContext else { return }
        let night = frame.colours.night
        switch frame.look {
        case .rest:
            break
        case .glow(let tone), .pulse(let tone):
            drawOrb(night.colour(tone, alert: alert), strength: frame.strength, in: rect, cg)
        case .ring(let progress):
            drawRing(progress, colour: night.them, in: rect, cg)
        }
    }

    /// The orb in `colour`: nearly solid out to two fifths of the glow's reach, then soft to its
    /// edge, as the icon's blurred orb is,
    /// with the icon's white core over it. `strength` scales the colour, not the core, so the
    /// pulse breathes the colour while the orb keeps its light.
    private static func drawOrb(_ colour: GlowColours.RGB, strength: Double, in rect: NSRect, _ cg: CGContext) {
        let side = rect.width
        let centre = CGPoint(x: rect.minX + side * orbCentre.x, y: rect.minY + side * orbCentre.y)
        let plate = plateRect(in: rect)
        cg.saveGState()
        defer { cg.restoreGState() }
        cg.addPath(CGPath(
            roundedRect: plate, cornerWidth: side * plateCorner, cornerHeight: side * plateCorner, transform: nil))
        cg.clip()
        let space = CGColorSpace(name: CGColorSpace.sRGB)!
        func c(_ rgb: GlowColours.RGB, _ alpha: Double) -> CGColor {
            CGColor(srgbRed: rgb.x, green: rgb.y, blue: rgb.z, alpha: alpha)
        }
        if let glow = CGGradient(
            colorsSpace: space,
            colors: [c(colour, strength), c(colour, strength * 0.92), c(colour, strength * 0.35), c(colour, 0)] as CFArray,
            locations: [0, 0.4, 0.75, 1]) {
            cg.drawRadialGradient(
                glow, startCenter: centre, startRadius: 0, endCenter: centre, endRadius: side * glowRadius, options: [])
        }
        let white = GlowColours.RGB(1, 1, 1)
        if let core = CGGradient(colorsSpace: space, colors: [c(white, 0.85), c(white, 0)] as CFArray, locations: [0, 1]) {
            cg.drawRadialGradient(
                core, startCenter: centre, startRadius: 0, endCenter: centre, endRadius: side * 0.055, options: [])
        }
    }

    /// The ring: a faint track all the way round, filled clockwise from the top to `progress`; with
    /// no number, evenly dashed, and still (an indeterminate ring that spun would be the constant
    /// motion the icon never has).
    private static func drawRing(_ progress: Double?, colour: GlowColours.RGB, in rect: NSRect, _ cg: CGContext) {
        let side = rect.width
        let ring = ringRect(in: rect)
        let corner = side * (plateCorner - ringInset)
        let path = clockwiseFromTop(ring, corner: corner)
        let length = 2 * (ring.width + ring.height) - 8 * corner + 2 * .pi * corner
        cg.saveGState()
        defer { cg.restoreGState() }
        cg.setLineWidth(side * ringWidth)
        cg.setLineCap(.butt)
        cg.addPath(path)
        cg.setStrokeColor(CGColor(srgbRed: 1, green: 1, blue: 1, alpha: 0.22))
        cg.strokePath()
        cg.setStrokeColor(CGColor(srgbRed: colour.x, green: colour.y, blue: colour.z, alpha: 1))
        if let progress {
            let filled = length * CGFloat(min(max(progress, 0), 1))
            guard filled > 0 else { return }
            cg.setLineDash(phase: 0, lengths: [filled, length + 1])
        } else {
            let dash = length / 32
            cg.setLineDash(phase: 0, lengths: [dash, dash])
        }
        cg.addPath(path)
        cg.strokePath()
    }

    /// A rounded rectangle starting at the top's middle and running clockwise, so a dash pattern
    /// fills it as a clock's hand sweeps.
    static func clockwiseFromTop(_ r: NSRect, corner: CGFloat) -> CGPath {
        let path = CGMutablePath()
        path.move(to: CGPoint(x: r.midX, y: r.maxY))
        path.addArc(tangent1End: CGPoint(x: r.maxX, y: r.maxY), tangent2End: CGPoint(x: r.maxX, y: r.minY), radius: corner)
        path.addArc(tangent1End: CGPoint(x: r.maxX, y: r.minY), tangent2End: CGPoint(x: r.minX, y: r.minY), radius: corner)
        path.addArc(tangent1End: CGPoint(x: r.minX, y: r.minY), tangent2End: CGPoint(x: r.minX, y: r.maxY), radius: corner)
        path.addArc(tangent1End: CGPoint(x: r.minX, y: r.maxY), tangent2End: CGPoint(x: r.midX, y: r.maxY), radius: corner)
        path.closeSubpath()
        return path
    }
}
