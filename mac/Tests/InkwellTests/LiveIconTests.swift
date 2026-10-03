// The icons that show the app's state: the menu-bar mark, and (LiveIcon) what the Dock tile and
// the menu-bar item show over the art while something is live.
import AppKit
import XCTest

@testable import Inkwell

@MainActor
final class StatusGlyphTests: XCTestCase {
    /// A template: macOS tints it for a light or dark menu bar, and for wallpaper tinting.
    func testTheMarkIsATemplateAtBothScales() throws {
        let image = StatusGlyph.image()
        XCTAssertTrue(image.isTemplate)
        XCTAssertEqual(image.size, NSSize(width: 18, height: 18))
        let widths = Set(image.representations.map(\.pixelsWide))
        XCTAssertEqual(widths, [18, 36], "drawn on its own grid at 1x and 2x")
        XCTAssertEqual(image.accessibilityDescription, "Inkwell")
    }

    /// The rim's straight edges and the orb's centre land on whole pixels: every pixel down the
    /// middle column is either fully inked or fully clear, never a soft half-pixel.
    func testTheRimAndTheOrbArePixelAligned() throws {
        for scale in [1, 2] {
            let rep = try XCTUnwrap(
                StatusGlyph.image().representations.first { $0.pixelsWide == 18 * scale } as? NSBitmapImageRep)
            let g = StatusGlyph.geometry(scale: scale)
            let side = 18 * scale
            let middle = side / 2
            func alpha(_ x: Int, _ y: Int) -> CGFloat { rep.colorAt(x: x, y: y)?.alphaComponent ?? -1 }
            for y in 0..<side {
                let rim = (g.inset..<(g.inset + g.stroke)).contains(y)
                    || ((side - g.inset - g.stroke)..<(side - g.inset)).contains(y)
                let orbTop = (side - g.orb) / 2
                let inOrb = (orbTop..<(orbTop + g.orb)).contains(y)
                // The orb's first and last rows are its curve's tip: soft by nature.
                if inOrb && (y == orbTop || y == orbTop + g.orb - 1) { continue }
                let expected: CGFloat = rim || inOrb ? 1 : 0
                XCTAssertEqual(alpha(middle, y), expected, accuracy: 0.001, "\(scale)x, row \(y)")
                XCTAssertEqual(alpha(y, middle), expected, accuracy: 0.001, "\(scale)x, column \(y)")
            }
        }
    }

    /// The same mark at both scales: the rim spans the same points, as a symbol's weights do.
    func testBothScalesDrawTheSameMark() {
        let one = StatusGlyph.geometry(scale: 1)
        let two = StatusGlyph.geometry(scale: 2)
        XCTAssertEqual(CGFloat(one.inset), CGFloat(two.inset) / 2)
        XCTAssertEqual(CGFloat(one.orb), CGFloat(two.orb) / 2)
        XCTAssertEqual(StatusGlyph.rimOuter, NSRect(x: 2, y: 2, width: 14, height: 14))
        XCTAssertEqual(StatusGlyph.orb, NSRect(x: 6, y: 6, width: 6, height: 6))
    }
}
