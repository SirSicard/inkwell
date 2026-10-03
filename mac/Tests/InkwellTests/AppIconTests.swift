// The app icon: Info.plist names it and the file it names is in mac/, at every size macOS asks
// for. Without it the Dock, Finder, About and the updater show a blank placeholder.
import AppKit
import XCTest

final class AppIconTests: XCTestCase {
    private var mac: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    }

    func testInfoPlistNamesAnIconThatIsThereAtEverySize() throws {
        let data = try Data(contentsOf: mac.appendingPathComponent("Info.plist"))
        let info = try XCTUnwrap(PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any])
        let name = try XCTUnwrap(info["CFBundleIconFile"] as? String, "Info.plist names no CFBundleIconFile")
        let icns = mac.appendingPathComponent("\(name).icns")
        let image = try XCTUnwrap(NSImage(contentsOf: icns), "\(icns.path) is missing or unreadable")
        let sizes = Set(image.representations.map { Int($0.pixelsWide) })
        for px in [16, 32, 64, 128, 256, 512, 1024] {
            XCTAssertTrue(sizes.contains(px), "the icon has no \(px) px image")
        }
    }
}
