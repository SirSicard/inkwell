// The menu-bar mark: the app icon, Halo rim, cut down to what reads at 18 pt: the plate's rim as a
// rounded square around the orb as a filled dot. It is a template, so macOS tints it for a light
// or dark menu bar, for wallpaper tinting and for an inactive display; the mark itself never takes
// a colour, and the live states draw over it (StatusItemController).
//
// Each scale is drawn on its own pixel grid rather than scaled from one drawing, so the rim's
// straight edges and the orb's centre fall on whole pixels at 1x and at 2x, crisp as a symbol is.
// The rim is filled as the space between two rounded squares, not stroked, so its edges are exact.
import AppKit

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
