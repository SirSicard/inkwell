// The INKWELL wordmark, rasterised with CoreText the way the prototype's `_uploadMark` does with a
// canvas: coverage only, the size of the canvas, row 0 at the top. The shader knocks it out of
// whatever sits under it: dark on paper, light on ink.
//
// The prototype asks for a 900-weight face (Geist). Geist is under the SIL OFL, which is not on
// this project's licence allowlist, so the app uses the system's heaviest face, SF Pro Black.
// The placement is the prototype's to the pixel: size, left edge, and a baseline where a browser
// canvas puts `textBaseline = 'top'`.
import AppKit
import CoreGraphics
import CoreText

/// The wordmark's face.
public enum WordmarkFont: Sendable, Equatable {
    /// SF Pro Black: the app's.
    case system
    /// A face by PostScript name, falling back to SF Pro Black when it is not installed. For
    /// comparisons with a reference rendered in that face.
    case named(String)
}

/// The rasterised wordmark.
public struct Wordmark: Sendable {
    /// Coverage, one byte per pixel, row 0 at the top. Empty when the wordmark is off.
    public var coverage: [UInt8]
    public var width: Int
    public var height: Int
    /// The face CoreText used (PostScript name), "" when off.
    public var fontName: String
    public var fontSize: Double
    /// The text's left edge and baseline, in canvas pixels from the top left.
    public var x: Double
    public var baselineFromTop: Double
    /// The text's advance width.
    public var textWidth: Double
    /// The box the letters can occupy (advance width by ascent and descent), in canvas pixels
    /// from the top left. Empty when off.
    public var box: CGRect

    /// The wordmark for a canvas of `width` x `height` pixels drawn at `pointWidth` points wide:
    /// the prototype sizes it from the canvas width in CSS pixels (`w`) and the backing scale.
    public static func rasterize(
        width: Int, height: Int, pointWidth: Double, font face: WordmarkFont = .system
    ) -> Wordmark {
        let scale = Double(width) / pointWidth
        let fontSize = jsRound(pointWidth * 0.158 * scale)
        let x = jsRound(fontSize * 0.62), top = jsRound(fontSize * 0.7)
        let font = Self.font(face, size: fontSize)
        // Skia snaps an axis-aligned baseline to whole pixels; so does this.
        let baseline = (top + emBoxAscent(font)).rounded()
        let attributes: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
            NSAttributedString.Key(kCTForegroundColorAttributeName as String): CGColor(gray: 0, alpha: 1),
        ]
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: "INKWELL", attributes: attributes))
        var ascent: CGFloat = 0, descent: CGFloat = 0
        let textWidth = Double(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
        var coverage = [UInt8](repeating: 0, count: width * height)
        let drawn = coverage.withUnsafeMutableBytes { buffer -> Bool in
            // Alpha only: the shader reads coverage from the alpha channel.
            guard let context = CGContext(
                data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                bytesPerRow: width, space: CGColorSpaceCreateDeviceGray(),
                bitmapInfo: CGImageAlphaInfo.alphaOnly.rawValue)
            else { return false }
            context.setShouldAntialias(true)
            context.setShouldSmoothFonts(false)
            // CoreGraphics y points up; memory row 0 is the top row.
            context.textPosition = CGPoint(x: x, y: Double(height) - baseline)
            CTLineDraw(line, context)
            context.flush()
            return true
        }
        guard drawn else {
            return Wordmark(coverage: [], width: width, height: height, fontName: "", fontSize: 0, x: 0,
                            baselineFromTop: 0, textWidth: 0, box: .zero)
        }
        let box = CGRect(x: x, y: baseline - Double(ascent), width: textWidth, height: Double(ascent + descent))
        return Wordmark(coverage: coverage, width: width, height: height,
                        fontName: CTFontCopyPostScriptName(font) as String, fontSize: fontSize, x: x,
                        baselineFromTop: baseline, textWidth: textWidth, box: box)
    }

    /// JavaScript's Math.round: floor(x + 0.5).
    static func jsRound(_ x: Double) -> Double { (x + 0.5).rounded(.down) }

    static func font(_ face: WordmarkFont, size: Double) -> CTFont {
        if case .named(let name) = face {
            let font = CTFontCreateWithName(name as CFString, size, nil)
            // CoreText substitutes a default face for a missing name; only an exact match counts.
            if (CTFontCopyPostScriptName(font) as String) == name { return font }
        }
        return NSFont.systemFont(ofSize: size, weight: .black) as CTFont
    }

    /// Where a browser canvas's `textBaseline = 'top'` sits above the baseline: the em box's top,
    /// which is the OS/2 typo ascender scaled so that ascender plus descender make one em,
    /// rounded to 1/64 px (Chrome's layout unit). Without an OS/2 table, hhea's ascent and descent.
    static func emBoxAscent(_ font: CTFont) -> Double {
        let size = Double(CTFontGetSize(font))
        var ascent = Double(CTFontGetAscent(font)), descent = Double(CTFontGetDescent(font))
        if let table = CTFontCopyTable(font, CTFontTableTag(kCTFontTableOS2), []) as Data?, table.count >= 72 {
            let unitsPerEm = Double(CTFontGetUnitsPerEm(font))
            let typoAscender = table.withUnsafeBytes { Int16(bigEndian: $0.loadUnaligned(fromByteOffset: 68, as: Int16.self)) }
            let typoDescender = table.withUnsafeBytes { Int16(bigEndian: $0.loadUnaligned(fromByteOffset: 70, as: Int16.self)) }
            ascent = Double(typoAscender) / unitsPerEm * size
            descent = -Double(typoDescender) / unitsPerEm * size
        }
        let height = ascent + descent
        guard height > 0, ascent >= 0, ascent <= height else { return size * 0.8 }
        return (ascent * size / height * 64).rounded() / 64
    }
}
