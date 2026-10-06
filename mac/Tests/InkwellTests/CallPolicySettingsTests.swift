// Per-app call recording in Settings > Meetings: the default for apps not chosen for (Always,
// Ask or Never) with its warning and hint, each app the core has seen with its own choice, and a
// stored list the core cannot read, started over only after the user agrees.
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
    return decoded
}

@MainActor
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
}

/// No app is installed: names come from the core, else from AppIdentity.
private struct NoApps: AppDirectory {
    func app(bundleID: String) -> (name: String, icon: NSImage)? { nil }
}

private let zoom = "us.zoom.xos"

private func callsEvent(_ apps: String, default policy: String = "ask", message: String? = nil, ref: String? = nil) -> InkEvent {
    var extra = message.map { #","message":"\#($0)""# } ?? ""
    extra += ref.map { #","ref":"\#($0)""# } ?? ""
    return event(#"{"type":"meetings.calls","default":"\#(policy)","apps":[\#(apps)]\#(extra)}"#)
}

@MainActor
final class CallPolicySettingsTests: XCTestCase {
    private let list = #"{"app":"us.zoom.xos","app_name":"Zoom","policy":"always","chosen":true,"seen_unix_ms":1759658400000},{"app":"com.microsoft.teams2","policy":"ask","chosen":false},{"app":"com.example.x","app_name":"an app","policy":"ask","chosen":false}"#

    func testTheListAndTheDefaultComeFromTheCore() {
        let sent = Sent()
        let calls = CallPolicyModel(send: sent.send, apps: NoApps())
        calls.load()
        XCTAssertEqual(sent.commands, [.meetingsCallsList(ref: "calls:1")])
        XCTAssertNil(calls.defaultPolicy, "not known until the core answers")
        calls.apply(callsEvent(list, ref: "calls:1"))
        XCTAssertEqual(calls.defaultPolicy, .ask)
        XCTAssertEqual(calls.rows.map(\.name), ["Zoom", "Microsoft Teams", "An app not on this Mac"], "never an identity")
        XCTAssertEqual(calls.rows.map(\.choice), [.always, .default, .default])
        XCTAssertEqual(calls.title(.default), "Default (Ask)")
        XCTAssertEqual(calls.policy(of: zoom), .always)
        XCTAssertNil(calls.policy(of: "com.unknown"))
    }

    func testAChoiceIsSentShownAndSettledByTheAnswer() {
        let sent = Sent()
        let calls = CallPolicyModel(send: sent.send, apps: NoApps())
        calls.apply(callsEvent(list))
        calls.choose(.never, for: "com.microsoft.teams2", from: .settings)
        XCTAssertEqual(sent.commands.last, .meetingsCallsSet(app: "com.microsoft.teams2", policy: "never", replaceUnreadable: false, ref: "calls:1"))
        XCTAssertEqual(calls.rows[1].choice, .never, "shown as made until the answer")
        XCTAssertEqual(calls.policy(of: "com.microsoft.teams2"), .never)
        // Back to the default.
        calls.choose(.default, for: zoom, from: .settings)
        XCTAssertEqual(sent.commands.last, .meetingsCallsSet(app: zoom, policy: "default", replaceUnreadable: false, ref: "calls:2"))
        XCTAssertEqual(calls.rows[0].choice, .default)
        calls.apply(callsEvent(#"{"app":"us.zoom.xos","app_name":"Zoom","policy":"ask","chosen":false},{"app":"com.microsoft.teams2","policy":"never","chosen":true}"#, ref: "calls:2"))
        XCTAssertEqual(calls.rows.map(\.choice), [.default, .never])
        // A failure is said, and the list read again.
        calls.choose(.always, for: zoom, from: .settings)
        calls.apply(event(#"{"type":"command.failed","command":"meetings.calls.set","id":"calls:3","message":"database is locked"}"#))
        XCTAssertEqual(calls.failure, "Couldn't save that: database is locked")
        XCTAssertEqual(calls.rows[0].choice, .default, "not shown as made")
        XCTAssertEqual(sent.commands.last, .meetingsCallsList(ref: "calls:4"))
    }

    func testTheDefaultIsASettingWithAWarningUnderAlwaysAndAHintUnderNever() {
        let sent = Sent()
        let calls = CallPolicyModel(send: sent.send, apps: NoApps())
        calls.apply(callsEvent(""))
        calls.setDefault(.always)
        XCTAssertEqual(sent.commands.last, .settingSet(.meetingsCallsDefault, "always"))
        XCTAssertEqual(CoreCommand.settingSet(.meetingsCallsDefault, "always").json,
                       #"{"cmd":"setting.set","id":"setting:meetings.calls.default","key":"meetings.calls.default","value":"always"}"#)
        XCTAssertEqual(calls.defaultPolicy, .always)
        calls.apply(event(#"{"type":"setting.value","key":"meetings.calls.default","value":"never"}"#))
        XCTAssertEqual(calls.defaultPolicy, .never)
        // The words: Always records without asking, so tell the people on the call; Never names
        // the way that still records.
        XCTAssertTrue(CallPolicyModel.alwaysWarning.contains("without asking"))
        XCTAssertTrue(CallPolicyModel.alwaysWarning.contains("Tell the people on the call"))
        XCTAssertTrue(CallPolicyModel.alwaysWarning.contains("asks instead"), "NotAlone is said in Settings too")
        XCTAssertTrue(CallPolicyModel.neverHint.contains("Record now"))
        for words in [CallPolicyModel.alwaysWarning, CallPolicyModel.neverHint, CallPolicyModel.defaultCaption] {
            for word in ["invisible", "undetectable", "hidden", "secret", "PC"] {
                XCTAssertFalse(words.contains(word), word)
            }
        }
        // A default the core refused: said, and read again.
        calls.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:meetings.calls.default","message":"database is locked"}"#))
        XCTAssertEqual(calls.failure, "Couldn't save the default: database is locked")
        XCTAssertEqual(sent.commands.last, .settingGet(.meetingsCallsDefault))
    }

    /// Over a list the core cannot read, a choice in Settings asks before starting it over, and
    /// the answer under Always says the default is Ask now.
    func testAnUnreadableListStartsOverOnlyAfterTheUserAgrees() {
        let sent = Sent()
        let calls = CallPolicyModel(send: sent.send, apps: NoApps())
        calls.apply(callsEvent("", default: "ask", message: "the stored choices cannot be read"))
        XCTAssertEqual(calls.unreadable, "the stored choices cannot be read")
        calls.choose(.never, for: zoom, from: .settings)
        XCTAssertTrue(sent.commands.isEmpty, "nothing sent before the user agrees")
        XCTAssertNotNil(calls.startingOver)
        calls.cancelStartOver()
        XCTAssertNil(calls.startingOver)
        calls.choose(.never, for: zoom, from: .settings)
        calls.confirmStartOver()
        XCTAssertEqual(sent.commands, [.meetingsCallsSet(app: zoom, policy: "never", replaceUnreadable: true, ref: "calls:1")])
        calls.apply(callsEvent(#"{"app":"us.zoom.xos","policy":"never","chosen":true}"#, default: "ask",
                          message: "the stored choices could not be read and were started over; the default is Ask now", ref: "calls:1"))
        XCTAssertNil(calls.unreadable, "readable again")
        XCTAssertEqual(calls.note, "the stored choices could not be read and were started over; the default is Ask now")
        // From the Drop there is no room to ask: it says where to choose.
        calls.apply(callsEvent("", message: "unreadable again"))
        calls.choose(.always, for: zoom, from: .drop)
        XCTAssertEqual(calls.dropFailure, CallPolicyModel.unreadableFromDrop)
    }

    func testTheLastCallCaption() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let now = Date(timeIntervalSince1970: 1_759_744_800) // 6 Oct 2025, 10:00 UTC
        XCTAssertEqual(CallPolicyModel.seenCaption(now.addingTimeInterval(-3600), calendar: calendar, now: now), "Last call today")
        XCTAssertNil(CallPolicyModel.seenCaption(nil))
        let earlier = CallPolicyModel.seenCaption(now.addingTimeInterval(-3 * 86_400), calendar: calendar, now: now)
        XCTAssertTrue(earlier?.hasPrefix("Last call ") == true)
        XCTAssertFalse(earlier?.contains("2025") == true, "this year's date has no year")
    }
}
