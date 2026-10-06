// Settings > AI's list of where polish may send (one consent per destination), and Revoke.
import Foundation
import InkBridge
import XCTest

@testable import Inkwell

private func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    guard let decoded = try? InkEvent.decode(Data(json.utf8)) else {
        XCTFail("not an event: \(json)", file: file, line: line)
        return .unknown(type: "")
    }
    return decoded
}

@MainActor
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
}

private func polishState(on: Bool, consents: [String] = [], ref: String? = nil) -> InkEvent {
    var fields = #""type":"consent.state","feature":"polish","on":\#(on),"allowed":true,"to":"on_device","name":"SystemLanguageModel.default","consents":[\#(consents.joined(separator: ","))]"#
    if let ref { fields += #","ref":"\#(ref)""# }
    return event("{\(fields)}")
}

private let groqConsent = #"{"to":"cloud","name":"Groq","endpoint":"https://api.groq.com/openai/v1"}"#
private let deviceConsent = #"{"to":"on_device"}"#

private func failed(_ command: String, id: String) -> InkEvent {
    event(#"{"type":"command.failed","command":"\#(command)","id":"\#(id)","message":"refused"}"#)
}

/// Settings > AI's list of where polish may send, with Revoke.
@MainActor
final class PolishConsentsTests: XCTestCase {
    func testEachDestinationIsListedAndRevokedOnItsOwn() throws {
        let sent = Sent()
        let consent = ConsentModel(feature: .polish, switchSettingID: PolishModel.settingID, send: sent.send)
        consent.apply(polishState(on: true, consents: [deviceConsent, groqConsent, #"{"to":"cloud"}"#]))
        let consents = try XCTUnwrap(consent.state?.consents)
        XCTAssertEqual(consents.map(\.label), ["Models on this Mac", "Groq"], "a cloud OK without its endpoint can't be named or revoked")
        XCTAssertEqual(consents.map(\.detail), ["Your words stay on this Mac.", "Your words leave this Mac for api.groq.com."])
        XCTAssertTrue(consent.state?.covers(ConsentModel.Destination(kind: .cloud(endpoint: "https://api.groq.com/openai/v1"), name: "")) ?? false)
        XCTAssertFalse(consent.state?.covers(ConsentModel.Destination(kind: .cloud(endpoint: "https://api.openai.com/v1"), name: "")) ?? true)

        consent.revoke(consents[1])
        XCTAssertEqual(sent.commands, [.consentRevoke(feature: .polish, to: .cloud, endpoint: "https://api.groq.com/openai/v1", ref: "consent.revoke:polish:1")])
        let json = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(sent.commands[0].json.utf8)) as? [String: String])
        XCTAssertEqual(json, ["cmd": "consent.revoke", "feature": "polish", "to": "cloud", "endpoint": "https://api.groq.com/openai/v1", "id": "consent.revoke:polish:1"])
        consent.revoke(consents[0])
        XCTAssertEqual(sent.commands.last, .consentRevoke(feature: .polish, to: .onDevice, endpoint: nil, ref: "consent.revoke:polish:2"))
        consent.apply(failed("consent.revoke", id: "consent.revoke:polish:2"))
        XCTAssertEqual(consent.problem, "Couldn't revoke that, so polish may still send there. Try again.")
        consent.apply(polishState(on: false, consents: [], ref: "consent.revoke:polish:2"))
        XCTAssertNil(consent.failure, "the core's answer shows what holds")
        XCTAssertEqual(consent.state?.consents, [])
    }

    /// A mode's OK is matched by its asker, never shown as the toggle's failure.
    func testAModesOKFailingIsNotTheTogglesFailure() {
        let sent = Sent()
        let consent = ConsentModel(feature: .polish, switchSettingID: PolishModel.settingID, send: sent.send)
        let ref = consent.allow(forMode: ConsentModel.Destination(kind: .onDevice, name: "x"))
        consent.apply(failed("consent.allow", id: ref))
        XCTAssertNil(consent.failure)
    }
}
