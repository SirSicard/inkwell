// The design tokens. The OS owns the chrome (the Liquid Glass sidebar and toolbar, native
// controls, the system fonts); the brand owns the content: paper, ink, and the ink zone.
//
// The six colours are the design canvas's. Everything else here derives from them, and each
// derivation says why.
import AppKit
import SwiftUI

/// One colour from the canvas, as its sRGB hex value.
struct Swatch: Equatable, Sendable {
    let hex: UInt32

    init(_ hex: UInt32) {
        self.hex = hex
    }

    var red: Double { Double((hex >> 16) & 0xFF) / 255 }
    var green: Double { Double((hex >> 8) & 0xFF) / 255 }
    var blue: Double { Double(hex & 0xFF) / 255 }

    var nsColor: NSColor {
        NSColor(srgbRed: red, green: green, blue: blue, alpha: 1)
    }
}

/// The canvas's palette.
enum Palette {
    /// The page: the content background by day, and the ink zone in both themes.
    static let paper = Swatch(0xF2EEE6)
    /// Text on paper, and the ink itself.
    static let ink = Swatch(0x16181F)
    /// The accent: native controls take it as their tint.
    static let sepia = Swatch(0x7E5431)
    /// Alerts: something needs the user.
    static let seal = Swatch(0xB23A26)
    /// The content background in dark mode.
    static let nightPaper = Swatch(0x121419)
    /// Secondary text on paper.
    static let muted = Swatch(0x625E57)
}

/// The palette mapped to what the screens paint.
enum Theme {
    /// Behind a screen's content: paper, or night paper in dark mode.
    static let surface = dynamic(light: Palette.paper.nsColor, dark: Palette.nightPaper.nsColor)
    /// Body text.
    static let text = dynamic(light: Palette.ink.nsColor, dark: Palette.paper.nsColor)
    /// Secondary text. Muted on night paper is 2.9:1 (WCAG), under the 4.5:1 text needs, so dark
    /// mode uses paper at 62 % (6.6:1 on night paper; muted on paper is 5.6:1).
    static let secondaryText = dynamic(
        light: Palette.muted.nsColor, dark: Palette.paper.nsColor.withAlphaComponent(0.62))
    /// The tint of native controls and selections (white on sepia is 6.6:1). Not a text colour in
    /// dark mode: sepia on night paper is 2.8:1.
    static let accent = Color(nsColor: Palette.sepia.nsColor)
    /// Something needs the user.
    static let alert = Color(nsColor: Palette.seal.nsColor)
    /// The ink zone and the rail are paper in both themes: the ink reads as ink on paper, and a
    /// dark drop on a dark page would vanish.
    static let inkZone = Color(nsColor: Palette.paper.nsColor)

    /// The window's own background, behind the SwiftUI content.
    static let windowBackground = dynamicNSColor(light: Palette.paper.nsColor, dark: Palette.nightPaper.nsColor)

    private static func dynamic(light: NSColor, dark: NSColor) -> Color {
        Color(nsColor: dynamicNSColor(light: light, dark: dark))
    }

    private static func dynamicNSColor(light: NSColor, dark: NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
        }
    }
}

/// The three system faces: SF Pro for the interface, New York for what the user reads as a
/// document (titles, summaries), SF Mono for timestamps. Text styles, so Dynamic Type and the
/// user's text size apply.
enum Typography {
    /// A screen's title (New York).
    static let screenTitle = Font.system(.largeTitle, design: .serif, weight: .semibold)
    /// A section heading inside a screen (New York).
    static let heading = Font.system(.title3, design: .serif, weight: .semibold)
    /// Interface text (SF Pro).
    static let body = Font.system(.body)
    /// Secondary interface text (SF Pro).
    static let caption = Font.system(.callout)
    /// Timestamps and durations (SF Mono).
    static let timestamp = Font.system(.caption, design: .monospaced)
}

/// Fixed measures of the layout.
enum Layout {
    /// The ink rail beside the content, outside Today (the design's 56 px rail).
    static let railWidth: CGFloat = 56
    /// Today's ink zone, with the wordmark: about a third of the default window's content (S2.5
    /// lays out the rest of Today).
    static let inkZoneWidth: CGFloat = 300
}
