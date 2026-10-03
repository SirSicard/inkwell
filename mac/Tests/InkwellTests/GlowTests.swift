// Glow on the Mac: the theme engine's settings and what they resolve to, Settings > AI's language
// model you bring, and text over the orb. The views are checked by hand.
import AppKit
import Foundation
import InkBridge
import InkRenderer
import SwiftUI
import XCTest

@testable import Inkwell

private func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    guard let decoded = try? InkEvent.decode(Data(json.utf8)) else {
        XCTFail("not an event: \(json)", file: file, line: line)
        return .unknown(type: "")
    }
    if case .undecodable = decoded {
        XCTFail("not this build's event: \(json)", file: file, line: line)
    }
    return decoded
}

private func fields(_ command: CoreCommand) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: Data(command.json.utf8))) as? [String: Any] ?? [:]
}

@MainActor
final class GlowThemeTests: XCTestCase {
    func testItReadsEveryKeyAndAppliesTheModeTheCoreHolds() {
        var sent: [CoreCommand] = []
        var applied: [NSAppearance.Name?] = []
        let theme = GlowTheme(send: { sent.append($0) }, applyAppearance: { applied.append($0?.name) })
        theme.load()
        XCTAssertEqual(sent.map { fields($0)["key"] as? String }, GlowTheme.keys.map(\.rawValue))
        XCTAssertEqual(theme.settings, GlowTheme.Settings(), "the defaults until the core answers")

        theme.apply(event(#"{"type":"setting.value","key":"appearance.mode","value":"dark"}"#))
        theme.apply(event(#"{"type":"setting.value","key":"appearance.dots.dark","value":"lagoon"}"#))
        theme.apply(event(##"{"type":"setting.value","key":"appearance.you.dark","value":"#336699"}"##))
        theme.apply(event(#"{"type":"setting.value","key":"appearance.edge_glow","value":"off"}"#))
        XCTAssertEqual(applied.last, .darkAqua)
        XCTAssertTrue(theme.isDark)
        XCTAssertEqual(theme.preset.id, "lagoon")
        XCTAssertEqual(GlowColours.hex(theme.dots.you), "#336699", "dark enough to stay as chosen")
        XCTAssertEqual(GlowColours.hex(theme.dots.them), "#ffbf47")
        XCTAssertFalse(theme.settings.edgeGlow)

        // An unset key is its default.
        theme.apply(event(#"{"type":"setting.value","key":"appearance.mode"}"#))
        XCTAssertEqual(theme.settings.mode, .system)
        XCTAssertEqual(applied.last, .some(nil), "system: the app follows the system again")
    }

    /// The speaker-name popover and polish's consent alert drew as dark glass over a Light window.
    /// A window (with its sheets) and a popover that follow the app's mode pin their own
    /// appearance to it, so what they present follows the window, not whatever else the app or
    /// the system holds: here the system's appearance is left alone (applyAppearance does
    /// nothing), and the window still reads the mode. Match system pins nothing.
    func testAWindowThatFollowsTheModePinsItsAppearanceToIt() {
        let theme = GlowTheme(send: { _ in }, applyAppearance: { _ in })
        let hosting = NSHostingController(rootView: Text("Inkwell").followsAppMode().environment(theme))
        let window = NSWindow(contentViewController: hosting)
        defer { window.close() }
        func shown() -> NSAppearance.Name? {
            hosting.view.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.1))
            return window.appearance?.name
        }
        for (mode, name) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua), ("light", .aqua)] {
            theme.apply(event(#"{"type":"setting.value","key":"appearance.mode","value":"\#(mode)"}"#))
            XCTAssertEqual(shown(), name, mode)
        }
        theme.apply(event(#"{"type":"setting.value","key":"appearance.mode","value":"system"}"#))
        XCTAssertNil(shown(), "Match system: the window follows the app, and the app the system")
        XCTAssertEqual(theme.appearance?.name, nil)
        theme.apply(event(#"{"type":"setting.value","key":"appearance.mode","value":"dark"}"#))
        XCTAssertEqual(theme.appearance?.name, .darkAqua, "what an app-modal alert is given")
    }

    func testPickingAPresetDropsThatModesOwnColours() {
        var sent: [CoreCommand] = []
        let theme = GlowTheme(send: { sent.append($0) }, applyAppearance: { _ in })
        theme.setMode(.light)
        theme.setYou("#123456")
        sent = []
        theme.setPreset("citrus")
        let writes = sent.map { (fields($0)["key"] as? String ?? "", fields($0)["value"] as? String ?? "") }
        XCTAssertEqual(writes.map(\.0), ["appearance.dots.light", "appearance.you.light", "appearance.them.light"])
        XCTAssertEqual(writes.map(\.1), ["citrus", "preset", "preset"])
        XCTAssertNil(theme.customYou)
    }

    /// Dragging in the colour panel writes value after value; the core's echo of an older one must
    /// not pull the control back.
    func testAnOlderEchoDoesNotUndoANewerWrite() {
        let theme = GlowTheme(send: { _ in }, applyAppearance: { _ in })
        theme.setMode(.light)
        theme.setYou("#111111")
        theme.setYou("#222222")
        theme.apply(event(##"{"type":"setting.value","key":"appearance.you.light","value":"#111111"}"##))
        XCTAssertEqual(theme.customYou, "#222222")
        theme.apply(event(##"{"type":"setting.value","key":"appearance.you.light","value":"#222222"}"##))
        XCTAssertEqual(theme.customYou, "#222222")
    }

    func testAFailedReadIsSaidNotHidden() {
        let theme = GlowTheme(send: { _ in }, applyAppearance: { _ in })
        theme.apply(event(#"{"type":"command.failed","command":"setting.get","id":"setting:appearance.mode","message":"setting.get: unknown key"}"#))
        XCTAssertNotNil(theme.failure)
    }
}

@MainActor
final class CloudModelTests: XCTestCase {
    private let providers = #"""
        {"type":"llm.providers","ref":"%@","local_only":true,"ready":false,"providers":[
          {"id":"openai","default_model":"gpt-x","endpoint":"https://api.openai.com/v1","custom_url":false,"needs_key":true,"has_key":true},
          {"id":"custom","default_model":"local","endpoint":"http://127.0.0.1:8080/v1","custom_url":true,"needs_key":false,"has_key":false}]}
        """#

    func testUsingAProviderOffThisMacSaysLocalOnlyGoesOff() throws {
        var sent: [CoreCommand] = []
        let cloud = CloudModel(send: { sent.append($0) })
        cloud.load()
        let ref = try XCTUnwrap(fields(try XCTUnwrap(sent.last))["id"] as? String)
        cloud.apply(event(String(format: providers, ref)))
        XCTAssertTrue(cloud.loaded)
        XCTAssertNil(cloud.selected, "nothing chosen yet")

        cloud.select("openai")
        XCTAssertTrue(cloud.selectedIsCloud)
        XCTAssertTrue(cloud.useNote.contains("turns local-only mode off"))
        cloud.use()
        let choose = fields(try XCTUnwrap(sent.last))
        XCTAssertEqual(choose["cmd"] as? String, "llm.choose")
        XCTAssertEqual(choose["provider"] as? String, "openai")
        XCTAssertEqual(choose["local_only"] as? String, "off", "the user's say-so travels with the choice")

        cloud.select("custom")
        XCTAssertFalse(cloud.selectedIsCloud, "a server on this Mac keeps local-only mode on")
        cloud.use()
        XCTAssertNil(fields(try XCTUnwrap(sent.last))["local_only"])
    }

    func testTheKeyIsSentOnceAndAnEmptyOneIsRefused() throws {
        var sent: [CoreCommand] = []
        let cloud = CloudModel(send: { sent.append($0) })
        cloud.apply(event(String(format: providers, "x")))
        cloud.select("openai")
        cloud.saveKey("   ")
        XCTAssertEqual(cloud.failure, "Type or paste the key first.")
        XCTAssertTrue(sent.isEmpty)
        cloud.saveKey("sk-test")
        XCTAssertEqual(sent.last?.name, "llm.key.save")
        XCTAssertFalse(Mirror(reflecting: cloud).children.contains { "\($0.value)".contains("sk-test") }, "never kept")
    }

    func testTheLocalOnlySwitchWritesTheSetting() throws {
        var sent: [CoreCommand] = []
        let cloud = CloudModel(send: { sent.append($0) })
        cloud.setLocalOnly(false)
        XCTAssertEqual(fields(try XCTUnwrap(sent.last))["key"] as? String, "llm.local_only")
        XCTAssertEqual(fields(try XCTUnwrap(sent.last))["value"] as? String, "off")
        cloud.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:llm.local_only","message":"the library is read-only"}"#))
        XCTAssertEqual(cloud.failure, "Couldn't change local-only mode: the library is read-only.")
    }

    func testWhatCountsAsThisMac() {
        XCTAssertTrue(CloudModel.isOnThisMac("http://localhost:11434/v1"))
        XCTAssertTrue(CloudModel.isOnThisMac("http://127.0.0.1:8080"))
        XCTAssertTrue(CloudModel.isOnThisMac("http://[::1]:8080/v1"))
        XCTAssertFalse(CloudModel.isOnThisMac("http://127.0.0.01"))
        XCTAssertFalse(CloudModel.isOnThisMac("http://user@localhost"))
        XCTAssertFalse(CloudModel.isOnThisMac("https://api.example.com"))
        XCTAssertFalse(CloudModel.isOnThisMac("localhost:8080"))
        XCTAssertTrue(CloudModel.keyWithheld(from: "http://10.0.0.2/v1"))
        XCTAssertFalse(CloudModel.keyWithheld(from: "https://10.0.0.2/v1"))
    }
}

/// The orb at rest in the dot preset's colours (the owner's report: "no matter which theme I have,
/// it's still grey in the background always"). The resting orb was the mode's idle colour in both
/// of its shades, whatever the preset; it now leans, softer than live, its first shade toward
/// yours and its second toward theirs.
@MainActor
final class OrbAtRestTests: XCTestCase {
    /// A colour's chroma: what is left once its grey (the mean of its channels) is taken away. The
    /// orb's highlight and grain add the same amount to every channel, so they drop out.
    private func chroma(_ c: SIMD3<Double>) -> SIMD3<Double> {
        c - SIMD3(repeating: (c.x + c.y + c.z) / 3)
    }

    private func cosine(_ a: SIMD3<Double>, _ b: SIMD3<Double>) -> Double {
        let la = (a * a).sum().squareRoot(), lb = (b * b).sum().squareRoot()
        return la > 0 && lb > 0 ? (a * b).sum() / (la * lb) : 0
    }

    /// The still frame the main window shows at rest, for one preset in one mode: how many of its
    /// solid pixels lean from the idle colour toward yours, how many toward theirs, out of how
    /// many, and their mean colour.
    private func rest(_ id: String, dark: Bool, pipeline: InkPipeline) throws
        -> (you: Int, them: Int, solid: Int, mean: SIMD3<Double>)
    {
        let palette = GlowColours.palette(preset: Glow.preset(id), you: nil, them: nil, dark: dark)
        let image = try InkSnapshot.render(
            .idle, t: 12, width: 208, height: 140, palette: palette, placement: Glow.Orb.main, voice: .silent,
            motion: false, pipeline: pipeline)
        let idle = chroma(SIMD3<Double>(palette.idle))
        let towardYou = chroma(SIMD3<Double>(palette.yA)) - idle
        let towardThem = chroma(SIMD3<Double>(palette.tA)) - idle
        var you = 0, them = 0, solid = 0, sum = SIMD3<Double>(0, 0, 0)
        for i in stride(from: 0, to: image.rgba.count, by: 4) where image.rgba[i + 3] >= 96 {
            let alpha = Double(image.rgba[i + 3])
            let c = SIMD3(Double(image.rgba[i]), Double(image.rgba[i + 1]), Double(image.rgba[i + 2])) / alpha
            solid += 1
            sum += c
            let lean = chroma(c) - idle
            // Past the rounding of an 8-bit premultiplied pixel at this alpha.
            guard (lean * lean).sum().squareRoot() >= 0.03 else { continue }
            if cosine(lean, towardYou) >= 0.8 { you += 1 }
            if cosine(lean, towardThem) >= 0.8 { them += 1 }
        }
        return (you, them, solid, solid > 0 ? sum / Double(solid) : sum)
    }

    /// Aurora rests green and violet, Lagoon teal and amber, in Light and in Dark: a fifth or more
    /// of the resting orb's pixels leans toward each of the preset's colours (a quarter to a half
    /// at the rest tint of 0.2; untinted, none but a few highlight pixels in Light), and the two
    /// presets' resting orbs differ.
    func testTheRestingOrbLeansTowardThePresetsColoursInBothModes() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let pipeline = try InkPipelineLoader.shared.wait().get()
        for dark in [false, true] {
            var means: [SIMD3<Double>] = []
            for id in ["aurora", "lagoon"] {
                let r = try rest(id, dark: dark, pipeline: pipeline)
                let label = "\(dark ? "dark" : "light") \(id): \(r.you) toward yours, \(r.them) toward theirs of \(r.solid)"
                XCTAssertGreaterThan(r.solid, 500, label)
                XCTAssertGreaterThanOrEqual(r.you * 5, r.solid, label)
                XCTAssertGreaterThanOrEqual(r.them * 5, r.solid, label)
                means.append(r.mean)
            }
            let apart = ((means[0] - means[1]) * (means[0] - means[1])).sum().squareRoot()
            XCTAssertGreaterThan(apart, 0.03, "\(dark ? "dark" : "light"): Aurora and Lagoon rest in different colours")
        }
    }
}

/// Text over the orb (a recorded-call test: "What's being said", the timestamps and the grey
/// settling lines washed out over Aurora's bright centre in Dark mode). The main window's orb sits
/// behind every screen's text, so wherever it draws, the mode's text and secondary text must stay
/// readable over it, with every preset in both modes.
@MainActor
final class OrbBehindTextTests: XCTestCase {
    /// WCAG's relative luminance of an sRGB colour, 0...1.
    private func luminance(_ c: SIMD3<Double>) -> Double {
        func linear(_ v: Double) -> Double { v <= 0.04045 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * linear(c.x) + 0.7152 * linear(c.y) + 0.0722 * linear(c.z)
    }

    private func contrast(_ a: Double, _ b: Double) -> Double {
        (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    func testALiveOrbDimsBehindTextAndAnOrbAtRestDoesNot() {
        XCTAssertEqual(OrbLayer.opacity(state: .idle, behindText: true, dimmed: false), 1, "at rest, as designed")
        for state in InkState.allCases where state.isLive {
            XCTAssertEqual(OrbLayer.opacity(state: state, behindText: true, dimmed: false), OrbLayer.liveBehindText, "\(state)")
            XCTAssertEqual(OrbLayer.opacity(state: state, behindText: false, dimmed: false), 1, "\(state): the first run's demo has no text over it")
            XCTAssertEqual(OrbLayer.opacity(state: state, behindText: true, dimmed: true), OrbLayer.liveBehindText, "\(state): never brighter for Increase Contrast")
        }
        XCTAssertEqual(OrbLayer.opacity(state: .idle, behindText: true, dimmed: true), 0.45)
    }

    /// The first run's orb sits beside its text on the sheet, at rest until the user speaks. The
    /// mode's idle colour is made to sit quietly behind text: beside it, on Light's paper, it was
    /// all but invisible (about 1.3:1; "its orb didn't show"). At rest on the sheet, at the
    /// opacity the sheet gives it (with Increase Contrast or Reduce Transparency too: no text is
    /// over it, so nothing dims it), the orb is a disc that stands out from the paper: a share of
    /// its pixels at 2:1 or more, not one bright speck.
    func testTheFirstRunsOrbAtRestShowsOnTheSheetInBothModes() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let pipeline = try InkPipelineLoader.shared.wait().get()
        for dark in [false, true] {
            let background = GlowColours.rgb(Glow.mode(dark: dark).background)
            let surface = luminance(background)
            for preset in Glow.presets {
                for solidSurfaces in [false, true] {
                    let style = OnboardingView.orbStyle(
                        GlowColours.palette(preset: preset, you: nil, them: nil, dark: dark), dark: dark,
                        solidSurfaces: solidSurfaces)
                    let opacity = Double(OrbLayer.opacity(state: .idle, behindText: false, dimmed: style.dimmed))
                    // The ready step's orb, 556 x 150 pt, at a quarter of its pixels.
                    let image = try InkSnapshot.render(
                        .idle, t: 12, width: 278, height: 75, palette: style.palette, placement: .centred,
                        voice: .silent, blotDepth: OrbLayer.blotDepth(behindText: false), pipeline: pipeline)
                    var standingOut = 0
                    for i in stride(from: 0, to: image.rgba.count, by: 4) where image.rgba[i + 3] > 0 {
                        let alpha = Double(image.rgba[i + 3]) / 255
                        let orb = SIMD3(Double(image.rgba[i]), Double(image.rgba[i + 1]), Double(image.rgba[i + 2])) / 255
                        let shown = luminance(orb * opacity + background * (1 - alpha * opacity))
                        if contrast(surface, shown) >= 2 { standingOut += 1 }
                    }
                    let label = "\(dark ? "dark" : "light") \(preset.id)\(solidSurfaces ? " solid" : "")"
                    // A disc, not a speck: Light reaches about 70 such pixels of 20,850, Dark about 600.
                    XCTAssertGreaterThanOrEqual(standingOut, 50, label)
                }
            }
        }
    }

    /// The orb drawn as the main window places it, loud voices on both sides, at a few moments
    /// (its noise and highlight move), composited over the mode's background at the opacity the
    /// window gives it: every pixel keeps text at 4.5:1 and secondary text at 3:1 or more. At rest
    /// that is the orb leaning toward the preset at Glow.restTint, undimmed: the worst is Dark's
    /// secondary text over Lagoon, 3.10:1 at 0.2 (2.96:1 at 0.25; 3.62:1 untinted).
    func testTextStaysReadableOverTheMainWindowsOrbWithEveryPresetInBothModes() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let pipeline = try InkPipelineLoader.shared.wait().get()
        for dark in [false, true] {
            let mode = Glow.mode(dark: dark)
            let background = GlowColours.rgb(mode.background)
            let text = luminance(GlowColours.rgb(mode.text))
            let secondary = luminance(GlowColours.rgb(mode.secondary))
            for preset in Glow.presets {
                let palette = GlowColours.palette(preset: preset, you: nil, them: nil, dark: dark)
                for state in InkState.allCases {
                    let opacity = Double(OrbLayer.opacity(state: state, behindText: true, dimmed: false))
                    var worstText = Double.infinity, worstSecondary = Double.infinity
                    for t in [3.0, 12, 27] {
                        // The orb scales with the window, so a small canvas holds the same colours.
                        let image = try InkSnapshot.render(
                            state, t: t, width: 208, height: 140, palette: palette, placement: Glow.Orb.main,
                            voice: .levels(near: 1, far: 1), blotDepth: OrbLayer.blotDepth(behindText: true),
                            pipeline: pipeline)
                        for i in stride(from: 0, to: image.rgba.count, by: 4) where image.rgba[i + 3] > 0 {
                            let alpha = Double(image.rgba[i + 3]) / 255
                            let orb = SIMD3(Double(image.rgba[i]), Double(image.rgba[i + 1]), Double(image.rgba[i + 2])) / 255
                            // Premultiplied over the background, as the window composites it.
                            let shown = luminance(orb * opacity + background * (1 - alpha * opacity))
                            worstText = min(worstText, contrast(text, shown))
                            worstSecondary = min(worstSecondary, contrast(secondary, shown))
                        }
                    }
                    let label = "\(dark ? "dark" : "light") \(preset.id) \(state)"
                    XCTAssertGreaterThanOrEqual(worstText, 4.5, label)
                    XCTAssertGreaterThanOrEqual(worstSecondary, 3, label)
                }
            }
        }
    }

    /// The blotting orb behind the main window (a recorded-call test: during the final pass it
    /// shrank to a hard-edged white disc sitting on Live's column divider). Behind text it keeps a
    /// soft edge: across the orb's middle row, its coverage never jumps by a quarter or more from
    /// one pixel to the next. The design's full blot, which the Drop keeps, is that disc.
    func testTheBlottingOrbBehindTextHasNoHardEdge() throws {
        try XCTSkipUnless(InkRenderer.isSupported, "no Metal device")
        let pipeline = try InkPipelineLoader.shared.wait().get()
        // The default window (1040 x 700) at a quarter.
        let width = 260, height = 175
        func steepestEdge(blotDepth: Double, dark: Bool) throws -> Int {
            let palette = GlowColours.palette(preset: Glow.preset("aurora"), you: nil, them: nil, dark: dark)
            var steepest = 0
            for t in [3.0, 12, 27] {
                let image = try InkSnapshot.render(
                    .blotting, t: t, width: width, height: height, palette: palette, placement: Glow.Orb.main,
                    voice: .silent, blotDepth: blotDepth, pipeline: pipeline)
                let y = Int((Double(height) * Glow.Orb.main.yFromTop).rounded())
                for x in 1..<width {
                    steepest = max(steepest, abs(Int(image.alpha(x, y)) - Int(image.alpha(x - 1, y))))
                }
            }
            return steepest
        }
        for dark in [false, true] {
            XCTAssertLessThan(try steepestEdge(blotDepth: OrbLayer.blotDepth(behindText: true), dark: dark), 64, "dark: \(dark)")
            XCTAssertGreaterThanOrEqual(try steepestEdge(blotDepth: OrbLayer.blotDepth(behindText: false), dark: dark), 128,
                                        "the full blot's drop is hard-edged: the check sees it")
        }
    }
}
