// Glow's design tokens, as the Mac shell uses them: the colours of each mode, the dot presets, the
// radii, the type sizes, the edge glow's strokes and where the orb sits.
//
// The values are design/tokens.json's, which both shells share, through its generated Swift file
// (Generated/GlowTokens.swift); this enum maps them to the shell's types, and is the one place the
// screens read them from.
import AppKit
import InkRenderer
import SwiftUI

enum Glow {
    /// One mode's colours.
    struct Mode: Equatable, Sendable {
        /// Behind the window's content.
        let background: Swatch
        /// Behind the window (the Drop's shadowed page, the canvas's board).
        let page: Swatch
        /// A card, at `cardAlpha` over the orb.
        let card: Swatch
        let cardAlpha: Double
        let text: Swatch
        let secondary: Swatch
        let border: Swatch
        /// A prominent button: filled with the text colour, labelled in the background's.
        let buttonFill: Swatch
        let buttonLabel: Swatch
        let alert: Swatch
        /// A card that needs the user.
        let alertCard: Swatch
        /// The blotted drop.
        let ink: Swatch
        /// The orb at rest.
        let idleOrb: Swatch
    }

    static let day = Mode(GlowTokens.light)
    static let night = Mode(GlowTokens.dark)

    static func mode(dark: Bool) -> Mode { dark ? night : day }

    /// A card's background blur, in points.
    static let cardBlur = CGFloat(GlowTokens.light.cardBlur)

    enum Radius {
        static let card = CGFloat(GlowTokens.Radius.card)
        static let window = CGFloat(GlowTokens.Radius.window)
        /// Pill buttons: fully round ends.
        static let pill = CGFloat(GlowTokens.Radius.pill)
    }

    /// The type roles' sizes, in points. Platform faces only: New York for display, SF Pro for the
    /// interface, SF Mono for times.
    enum Size {
        static let greeting = CGFloat(GlowTokens.TypeRole.greeting)
        static let screenTitle = CGFloat(GlowTokens.TypeRole.screenTitle)
        static let heading = CGFloat(GlowTokens.TypeRole.heading)
        static let body = CGFloat(GlowTokens.TypeRole.body)
        static let caption = CGFloat(GlowTokens.TypeRole.caption)
        static let eyebrow = CGFloat(GlowTokens.TypeRole.eyebrow)
    }

    /// A pair of dot colours: yours and theirs.
    struct Preset: Equatable, Identifiable, Sendable {
        let id: String
        let name: String
        let you: Swatch
        let them: Swatch
    }

    static let presets = GlowTokens.presets.map {
        Preset(id: $0.id, name: $0.name, you: Swatch($0.you.hex), them: Swatch($0.them.hex))
    }

    /// The preset with `id`, else the first (the default).
    static func preset(_ id: String?) -> Preset {
        presets.first { $0.id == id } ?? presets[0]
    }

    /// The edge glow: its strokes wide to narrow, the blend band, the lean and the flow.
    static let edge = EdgeGlowStyle(
        strokes: GlowTokens.EdgeGlow.strokes.map { .init(width: $0.width, alpha: $0.alpha) },
        band: GlowTokens.EdgeGlow.blendBand, lean: GlowTokens.EdgeGlow.lean,
        flowAmplitude: GlowTokens.EdgeGlow.flowAmplitude, flowSpeed: GlowTokens.EdgeGlow.flowRate,
        cornerRadius: GlowTokens.Radius.window)

    /// Where the orb sits behind the main window, and in the Drop.
    enum Orb {
        static let main = placement(GlowTokens.Orb.main)
        static let drop = placement(GlowTokens.Orb.drop)
        /// Where the main window's orb wanders, around `main` (x 0.56, y 0.26): its centre stays
        /// within this region of the window, which keeps it clear of the sidebar and mostly in the
        /// top half, behind the screens' headings. OrbBehindTextTests renders the orb at each corner
        /// too. The Drop's orb and the first run's do not wander.
        static let wander = OrbWander.Bounds(x: 0.46...0.68, y: 0.18...0.36)

        private static func placement(_ p: GlowOrbPlacement) -> OrbPlacement {
            OrbPlacement(x: p.x, yFromTop: p.y, unit: p.unit)
        }
    }
}

extension Glow.Mode {
    /// The generated palette, as the shell's swatches; the card's opacity apart.
    init(_ p: GlowPalette) {
        func s(_ c: GlowColor) -> Swatch { Swatch(c.hex) }
        self.init(
            background: s(p.background), page: s(p.page), card: s(p.card), cardAlpha: p.card.alpha,
            text: s(p.text), secondary: s(p.secondary), border: s(p.border), buttonFill: s(p.buttonFill),
            buttonLabel: s(p.buttonLabel), alert: s(p.alert), alertCard: s(p.alertCard), ink: s(p.ink),
            idleOrb: s(p.idleOrb))
    }
}

// MARK: - Resolving colours

/// The dot colours, resolved the same way on both shells: the mode's preset, a colour of the
/// user's own in its place, fitted so it shows on the mode's background, and a lighter partner for
/// the orb's second shade.
enum GlowColours {
    typealias RGB = SIMD3<Double>

    static func rgb(_ swatch: Swatch) -> RGB { RGB(swatch.red, swatch.green, swatch.blue) }

    /// "#rrggbb" (lowercase, as the settings store it), or nil.
    static func parse(_ hex: String?) -> RGB? {
        guard let hex, hex.count == 7, hex.first == "#",
              hex.dropFirst().allSatisfy({ $0.isHexDigit && !$0.isUppercase }),
              let value = UInt32(hex.dropFirst(), radix: 16)
        else { return nil }
        return rgb(Swatch(value))
    }

    /// "#rrggbb", lowercase.
    static func hex(_ c: RGB) -> String {
        let bytes = [c.x, c.y, c.z].map { Int((min(1, max(0, $0)) * 255).rounded()) }
        return "#" + bytes.map { String(format: "%02x", $0) }.joined()
    }

    private typealias Fit = GlowTokens.Fit

    static func luminance(_ c: RGB) -> Double { Fit.lumaRed * c.x + Fit.lumaGreen * c.y + Fit.lumaBlue * c.z }

    /// Too dark for night is lifted; too pale for day is deepened.
    static func fit(_ c: RGB, dark: Bool) -> RGB {
        if dark, luminance(c) < Fit.darkLiftBelow { return c + (RGB(1, 1, 1) - c) * Fit.darkLift }
        if !dark, luminance(c) > Fit.lightDimAbove { return c * Fit.lightDim }
        return c
    }

    /// The orb's second shade: the same colour, lighter.
    static func partner(_ c: RGB) -> RGB { c + (RGB(1, 1, 1) - c) * Fit.partnerLift }

    /// The colours one mode shows: you and them as chosen and fitted.
    static func dots(preset: Glow.Preset, you: String?, them: String?, dark: Bool) -> (you: RGB, them: RGB) {
        let you = parse(you) ?? rgb(preset.you)
        let them = parse(them) ?? rgb(preset.them)
        return (fit(you, dark: dark), fit(them, dark: dark))
    }

    /// The orb's palette for one mode.
    static func palette(preset: Glow.Preset, you: String?, them: String?, dark: Bool) -> OrbPalette {
        let dots = dots(preset: preset, you: you, them: them, dark: dark)
        let mode = Glow.mode(dark: dark)
        func f(_ c: RGB) -> SIMD3<Float> { SIMD3<Float>(c) }
        return OrbPalette(
            yA: f(dots.you), yB: f(partner(dots.you)), tA: f(dots.them), tB: f(partner(dots.them)),
            idle: f(rgb(mode.idleOrb)), ink: f(rgb(mode.ink)), dark: dark, restTint: Float(restTint))
    }

    /// How far the orb at rest leans from the mode's idle colour toward the dots, so each preset
    /// clearly shows at rest. Chosen with OrbLayer.restBehindText (0.7) by OrbBehindTextTests:
    /// with every preset in both modes, text keeps 4.5:1 and secondary text 3:1 or more. Dark's
    /// secondary text has the least room: 3.27:1 at 0.6 and 0.7. When chosen (a sweep, not kept as
    /// a test): 3.05:1 at 0.6 and 0.75, and 2.19:1 at 0.6 undimmed, where text alone is still
    /// 4.72:1. Increase Contrast dims the resting orb to 0.45, so its tint shows less there.
    static let restTint = 0.6

    static func color(_ c: RGB) -> Color {
        Color(nsColor: nsColor(c))
    }

    static func nsColor(_ c: RGB) -> NSColor {
        NSColor(srgbRed: c.x, green: c.y, blue: c.z, alpha: 1)
    }
}
