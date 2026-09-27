// In-app updates: when the bundle turns them on, and what the shipped Info.plist asks Sparkle for.
// No test starts a real updater: that would fetch the feed.
import Foundation
import XCTest

@testable import Inkwell

final class UpdateAvailabilityTests: XCTestCase {
    private let feed = "https://github.com/example/inkwell/releases/latest/download/appcast.xml"
    /// 32 bytes, base64: the shape of an EdDSA (Ed25519) public key.
    private let key = Data((0..<32).map { UInt8($0) }).base64EncodedString()

    private func info(feed: String? = nil, key: String? = nil) -> [String: Any] {
        var info: [String: Any] = ["CFBundleIdentifier": "com.inkwell.app"]
        info["SUFeedURL"] = feed
        info["SUPublicEDKey"] = key
        return info
    }

    func testAnHTTPSFeedAndAKeyTurnUpdatesOn() {
        XCTAssertEqual(UpdateAvailability(infoDictionary: info(feed: feed, key: key)),
                       .on(feed: URL(string: feed)!))
    }

    func testUpdatesStayOffUntilTheKeyExists() {
        XCTAssertEqual(UpdateAvailability(infoDictionary: info(feed: feed, key: "")), .off(.noKey))
        XCTAssertEqual(UpdateAvailability(infoDictionary: info(feed: feed, key: "  ")), .off(.noKey))
        XCTAssertEqual(UpdateAvailability(infoDictionary: info(feed: feed)), .off(.noKey))
    }

    func testAKeyThatIsNotThirtyTwoBytesOfBase64IsRefused() {
        let short = Data(repeating: 1, count: 31).base64EncodedString()
        let long = Data(repeating: 1, count: 64).base64EncodedString()
        // Whitespace pasted around a key: Sparkle would not read it, so it is refused here too.
        for bad in [short, long, "not base64 at all", String(key.dropLast(2)), " \(key)", "\(key)\n"] {
            XCTAssertEqual(UpdateAvailability(infoDictionary: info(feed: feed, key: bad)), .off(.malformedKey), bad)
        }
    }

    func testTheFeedMustBeAnHTTPSURL() {
        for bad in [nil, "", "http://example.com/appcast.xml", "file:///tmp/appcast.xml", "appcast.xml"] {
            XCTAssertEqual(UpdateAvailability(infoDictionary: info(feed: bad, key: key)), .off(.noFeed),
                           bad ?? "nil")
        }
    }

    func testWithoutABundleThereAreNoUpdates() {
        XCTAssertEqual(UpdateAvailability(infoDictionary: nil), .off(.notABundle))
        XCTAssertEqual(UpdateAvailability(infoDictionary: ["SUFeedURL": feed, "SUPublicEDKey": key]),
                       .off(.notABundle), "no bundle identifier: `swift run`, not an app")
    }

    func testEveryReasonExplainsItself() {
        for reason in [UpdateAvailability.Reason.notABundle, .noFeed, .noKey, .malformedKey] {
            XCTAssertFalse(reason.explanation.isEmpty, "\(reason)")
        }
    }
}

final class ShippedUpdateSettingsTests: XCTestCase {
    /// mac/Info.plist, the one build-mac.sh copies into the bundle.
    private func shippedInfo() throws -> [String: Any] {
        let plist = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Info.plist")
        let data = try Data(contentsOf: plist)
        return try XCTUnwrap(PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any])
    }

    func testTheFeedIsTheLatestReleasesAppcast() throws {
        let info = try shippedInfo()
        XCTAssertEqual(info["SUFeedURL"] as? String,
                       "https://github.com/SirSicard/inkwell/releases/latest/download/appcast.xml")
    }

    func testTheShippedKeyIsThePlaceholderOrAKeyNeverSomethingInBetween() throws {
        // The placeholder keeps updates off until the maintainer commits the key's public half; a
        // key pasted wrong would turn them off just as quietly, so it fails here instead.
        let availability = UpdateAvailability(infoDictionary: try shippedInfo())
        switch availability {
        case .on, .off(.noKey): break
        default: XCTFail("the shipped Info.plist turns updates off: \(availability)")
        }
    }

    func testSparkleChecksTheArchiveAndTheFeedAndSendsNoProfile() throws {
        let info = try shippedInfo()
        XCTAssertEqual(info["SUVerifyUpdateBeforeExtraction"] as? Bool, true)
        XCTAssertEqual(info["SURequireSignedFeed"] as? Bool, true)
        XCTAssertEqual(info["SUEnableSystemProfiling"] as? Bool, false)
        // Not sandboxed: Sparkle's XPC services stay unused, so nothing opts into them.
        XCTAssertNil(info["SUEnableInstallerLauncherService"])
        XCTAssertNil(info["SUEnableDownloaderService"])
        // The user is asked before the first automatic check; nothing decides it for them.
        XCTAssertNil(info["SUEnableAutomaticChecks"])
        XCTAssertNil(info["SUAutomaticallyUpdate"])
    }
}

@MainActor
final class UpdatesTests: XCTestCase {
    func testWithUpdatesOffThereIsNoUpdaterAndNoMenuItem() {
        let updates = Updates(infoDictionary: ["CFBundleIdentifier": "com.inkwell.app"])
        XCTAssertEqual(updates.availability, .off(.noFeed))
        XCTAssertFalse(updates.canCheck)
        XCTAssertNil(updates.makeMenuItem())
        XCTAssertFalse(updates.checksAutomatically)
        updates.checksAutomatically = true
        XCTAssertFalse(updates.checksAutomatically, "no updater to keep the setting")
        updates.checkForUpdates() // nothing to do, and nothing to crash on
    }
}
