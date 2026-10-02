// The design tokens as the screens paint them. The OS owns the chrome (the Liquid Glass sidebar
// and toolbar, native controls, the system fonts); Glow owns the content: the background, the
// translucent cards floating over the orb, the type, and the colours of each mode (Glow.swift).
//
// Every colour here is dynamic: it resolves against the appearance it is drawn in, which follows
// the theme's mode (GlowTheme sets the app's appearance). The dot colours (yours and theirs) are
// not here: they depend on the user's settings, so GlowTheme resolves them. Text never uses them.
import AppKit
import SwiftUI

/// One colour, as its sRGB hex value.
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

/// The mode's tokens mapped to what the screens paint.
enum Theme {
    /// Behind a screen's content: the window's background (the orb draws over it).
    static let surface = color(\.background)
    /// Body text.
    static let text = color(\.text)
    /// Secondary text.
    static let secondaryText = color(\.secondary)
    /// Hairlines and borders.
    static let border = color(\.border)
    /// A translucent card over the orb.
    static let card = Color(nsColor: dynamic { mode in mode.card.nsColor.withAlphaComponent(mode.cardAlpha) })
    /// A card, solid: Increase Contrast and Reduce Transparency.
    static let solidCard = color(\.card)
    /// A prominent button: text-coloured fill, background-coloured label.
    static let buttonFill = color(\.buttonFill)
    static let buttonLabel = color(\.buttonLabel)
    /// Something needs the user.
    static let alert = color(\.alert)
    /// A card that needs the user.
    static let alertCard = color(\.alertCard)
    /// The page behind a floating surface (the Drop's shadow side).
    static let page = color(\.page)

    /// The window's own background, behind the SwiftUI content.
    static let windowBackground = dynamic { $0.background.nsColor }

    /// Words on the accent: a list's selected row, which AppKit fills with the accent while the
    /// list has the keyboard. The app's accent is the button fill (Info.plist), so these are the
    /// button label; a user's own accent wins over the app's, so the label is chosen against the
    /// accent actually drawn. (SwiftUI's own selected-row words are white, unreadable on the night
    /// fill.)
    static let onAccent = Color(nsColor: NSColor(name: nil) { appearance in
        var accent = GlowColours.rgb(Glow.mode(dark: appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua).buttonFill)
        appearance.performAsCurrentDrawingAppearance {
            if let c = NSColor.controlAccentColor.usingColorSpace(.sRGB) {
                accent = GlowColours.RGB(c.redComponent, c.greenComponent, c.blueComponent)
            }
        }
        return label(onAccent: accent).nsColor
    })

    /// The button label that reads on `accent`: night's (dark) on a light accent, day's (light)
    /// on a dark one.
    static func label(onAccent accent: GlowColours.RGB) -> Swatch {
        GlowColours.luminance(accent) > 0.5 ? Glow.night.buttonLabel : Glow.day.buttonLabel
    }

    /// The mode's colour at `path`, following the appearance it is drawn in.
    static func color(_ path: any KeyPath<Glow.Mode, Swatch> & Sendable) -> Color {
        Color(nsColor: dynamic { $0[keyPath: path].nsColor })
    }

    /// An NSColor that resolves per appearance: night tokens in a dark one, day tokens otherwise.
    static func dynamic(_ make: @escaping @Sendable (Glow.Mode) -> NSColor) -> NSColor {
        NSColor(name: nil) { appearance in
            make(Glow.mode(dark: appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua))
        }
    }
}

/// The type roles, platform faces only: New York for display (greetings, titles, what the user
/// reads as a document), SF Pro for the interface, SF Mono for timestamps. Fixed sizes, the
/// tokens' own.
enum Typography {
    /// Today's greeting (New York).
    static let greeting = Font.system(size: Glow.Size.greeting, design: .serif)
    /// A screen's title (New York).
    static let screenTitle = Font.system(size: Glow.Size.screenTitle, design: .serif)
    /// A section heading inside a screen (New York).
    static let heading = Font.system(size: Glow.Size.heading, design: .serif)
    /// Interface text (SF Pro).
    static let body = Font.system(size: Glow.Size.body)
    /// Secondary interface text (SF Pro).
    static let caption = Font.system(size: Glow.Size.caption)
    /// A small label over a section (SF Pro).
    static let eyebrow = Font.system(size: Glow.Size.eyebrow, weight: .medium)
    /// Timestamps and durations (SF Mono).
    static let timestamp = Font.system(size: Glow.Size.eyebrow, design: .monospaced)
}
