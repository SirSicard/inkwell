// Glow on the Mac: the theme engine's settings and what they resolve to, and Settings > AI's
// language model you bring. The views are checked by hand.
import AppKit
import Foundation
import InkBridge
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
