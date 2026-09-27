// The application object is InkwellApplication (a Quit nothing on screen can refuse), and the
// shipped Info.plist says so.
import AppKit
import XCTest

@testable import Inkwell

final class InkwellApplicationTests: XCTestCase {
    /// mac/Info.plist, the one build-mac.sh copies into the bundle.
    private func shippedInfo() throws -> [String: Any] {
        let plist = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Info.plist")
        let data = try Data(contentsOf: plist)
        return try XCTUnwrap(PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any])
    }

    func testTheShippedInfoPlistNamesTheApplicationClass() throws {
        XCTAssertEqual(try shippedInfo()["NSPrincipalClass"] as? String, NSStringFromClass(InkwellApplication.self))
    }
}
