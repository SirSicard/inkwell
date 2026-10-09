import AppKit
import XCTest
@testable import Inkwell

@MainActor
final class DropAppearanceTests: XCTestCase {
    /// contentTintColor alone left Record black on a dark bezel in Light and Stop white on
    /// a white bezel in Dark. Inspect the actual attributed title and configured bezel.
    func testProminentCallActionsHaveAnExplicitReadableTitleInBothModes() throws {
        for mode in [NSAppearance.Name.aqua, .darkAqua] {
            for text in [
                DropText(title: "Zoom opened the microphone", detail: DropText.consentLine,
                         actions: [.record(app: "synthetic.zoom"), .dismiss(app: "synthetic.zoom")]),
                DropText(title: "Recording Zoom automatically", detail: DropText.autoReminder("Zoom"),
                         tone: .recording, actions: [.stop, .stopAndDelete], recordingName: "Zoom")
            ] {
                let view = DropContentView()
                view.appearance = NSAppearance(named: mode)
                view.show(text)
                let button = try XCTUnwrap(view.shownButtons.first)
                let label = try XCTUnwrap(button.attributedTitle.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor)
                let bezel = try XCTUnwrap(button.bezelColor)
                var ratio = 0.0
                var failure: Error?
                view.effectiveAppearance.performAsCurrentDrawingAppearance {
                    do {
                        func luminance(_ color: NSColor) throws -> Double {
                            let rgb = try XCTUnwrap(color.usingColorSpace(.sRGB))
                            func linear(_ channel: CGFloat) -> Double {
                                let c = Double(channel)
                                return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
                            }
                            return 0.2126 * linear(rgb.redComponent) + 0.7152 * linear(rgb.greenComponent) + 0.0722 * linear(rgb.blueComponent)
                        }
                        let a = try luminance(label), b = try luminance(bezel)
                        ratio = (max(a, b) + 0.05) / (min(a, b) + 0.05)
                    } catch { failure = error }
                }
                if let failure { throw failure }
                XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(mode): \(button.title)")
            }
        }
    }
}
