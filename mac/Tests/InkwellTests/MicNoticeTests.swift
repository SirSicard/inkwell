// What the store keeps, and the Drop and Live say, when the chosen mic isn't connected
// (audio.input_fallback) or a meeting's mic goes (meeting.mic_switched).
import Foundation
import InkBridge
import InkRenderer
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

/// An `audio.devices` answer: the Mac's mic and AirPods, with `input` chosen and `using` as given.
private func devices(
    input: String = "auto", using: String = #"{"id":"mbp","name":"MacBook Pro Microphone","transport":"built_in","reason":"default_input"}"#,
    wanted: String? = nil, type: String = "audio.devices"
) -> InkEvent {
    let wantedField = wanted.map { #","wanted":\#($0)"# } ?? ""
    return event(#"""
    {"type":"\#(type)","ref":"sound.devices","input":"\#(input)"\#(wantedField),
     "inputs":[{"id":"mbp","name":"MacBook Pro Microphone","transport":"built_in","is_default":true},
               {"id":"pods","name":"AirPods Pro","transport":"bluetooth","is_default":false},
               {"id":"odd","name":"Loopback","transport":"other","is_default":false}],
     "automatic":{"id":"mbp","name":"MacBook Pro Microphone","transport":"built_in","reason":"built_in_for_bluetooth_output"},
     "using":\#(using)}
    """#)
}

/// What the store keeps of a chosen mic that is missing, and of a meeting's mic that went.
@MainActor
final class MicNoticeStoreTests: XCTestCase {
    func testAFallbackIsKeptForItsTakeAndGoesWithItOrWhenTheChosenMicIsBack() {
        let store = CoreStore()
        store.apply([event(#"{"type":"audio.input_fallback","wanted":{"id":"pods","name":"AirPods Pro","transport":"bluetooth"},"mic_name":"MacBook Pro Microphone","mic_transport":"built_in"}"#)])
        XCTAssertEqual(store.micFallback, CoreStore.MicFallback(wanted: "AirPods Pro", using: "MacBook Pro Microphone"))
        store.apply([event(#"{"type":"dictation.started","take":1,"edit":false}"#)])
        XCTAssertNotNil(store.micFallback, "said during the take it opened for")
        store.apply([event(#"{"type":"dictation.discarded","take":1,"reason":"too_short"}"#)])
        XCTAssertNil(store.micFallback, "and let go of after it")

        store.apply([event(#"{"type":"audio.input_fallback","wanted":{"id":"pods"},"mic_name":"MacBook Pro Microphone","mic_transport":"built_in"}"#)])
        XCTAssertEqual(store.micFallback?.wanted, nil, "a choice stored without its name")
        store.apply([devices(input: "pods", using: #"{"id":"mbp","name":"MacBook Pro Microphone","transport":"built_in","reason":"chosen_missing"}"#, type: "audio.devices_changed")])
        XCTAssertNotNil(store.micFallback, "still missing")
        store.apply([devices(input: "pods", using: #"{"id":"pods","name":"AirPods Pro","transport":"bluetooth","reason":"chosen"}"#, type: "audio.devices_changed")])
        XCTAssertNil(store.micFallback, "the chosen mic is back")
    }

    func testTheCoreStoppingLetsGoOfAFallback() {
        let store = CoreStore()
        store.apply([event(#"{"type":"audio.input_fallback","wanted":{"id":"pods","name":"AirPods Pro"},"mic_name":"M","mic_transport":"usb"}"#)])
        store.apply([event(#"{"type":"core.stopped"}"#)])
        XCTAssertNil(store.micFallback)
    }

    func testAMeetingsMicThatWentIsKeptForLiveAndMarksTheLineItCameAt() {
        let store = CoreStore()
        store.apply([event(#"{"type":"meeting.started","record":"r","mic_name":"AirPods Pro","mic_transport":"bluetooth","mic_reason":"chosen","far_end":"app"}"#)])
        store.apply([event(#"{"type":"meeting.mic_switched","record":"r","from_name":"AirPods Pro","from_transport":"bluetooth","mic_name":"MacBook Pro Microphone","mic_transport":"built_in","mic_reason":"chosen_missing"}"#)])
        XCTAssertEqual(store.meeting?.micSwitch, CoreStore.MicSwitch(from: "AirPods Pro", to: "MacBook Pro Microphone", atLine: 0))
        XCTAssertEqual(store.meeting?.micName, "MacBook Pro Microphone")
        XCTAssertEqual(store.meeting?.micReason, .chosenMissing)
        store.apply([event(#"{"type":"meeting.final","record":"r","channel":"far","start_ms":0,"end_ms":900,"text":"Hello"}"#)])
        XCTAssertNotNil(store.meeting?.micSwitch, "kept: Live says it for the rest of the meeting")
        let meeting = try? XCTUnwrap(store.meeting)
        // The Drop says it only until a line comes after it, from either side (a listening-only
        // meeting would otherwise never show the other side's lines again).
        let drop = DropText.for(.meeting, dictation: .idle, meeting: meeting, offer: nil, systemAudioOff: false)
        XCTAssertEqual(drop.detail, "Hello")
    }
}

/// The Drop's lines and Live's mic line for those.
@MainActor
final class MicNoticeTextTests: XCTestCase {
    private let fallback = CoreStore.MicFallback(wanted: "AirPods Pro", using: "MacBook Pro Microphone")

    func testATakeOnAStandInMicSaysSoUntilItsWordsCome() {
        let listening = DropText.for(
            .dictating, dictation: .listening, live: .init(take: 1, edit: false, mode: nil, app: "Notes"),
            meeting: nil, offer: nil, systemAudioOff: false, micFallback: fallback)
        XCTAssertEqual(listening.title, "Dictating · Notes")
        XCTAssertEqual(listening.detail, "AirPods Pro isn't connected. Using MacBook Pro Microphone.")
        let words = DropText.for(
            .dictating, dictation: .listening, live: .init(take: 1, edit: false, mode: nil, app: nil, partial: "hello there"),
            meeting: nil, offer: nil, systemAudioOff: false, micFallback: fallback)
        XCTAssertEqual(words.detail, "hello there", "the words win")
        let none = DropText.for(
            .dictating, dictation: .listening, live: .init(take: 1, edit: false, mode: nil, app: nil),
            meeting: nil, offer: nil, systemAudioOff: false)
        XCTAssertEqual(none.detail, "Listening")
        XCTAssertEqual(DropText.fallbackLine(.init(wanted: nil, using: "USB Mic")), "Your chosen mic isn't connected. Using USB Mic.")
    }

    func testAMeetingSaysItsStandInMicAndAMicThatWent() {
        var meeting = CoreStore.LiveMeeting(record: "r")
        meeting.appName = "Zoom"
        let waiting = DropText.for(.meeting, dictation: .idle, meeting: meeting, offer: nil, systemAudioOff: false, micFallback: fallback)
        XCTAssertEqual(waiting.detail, "AirPods Pro isn't connected. Using MacBook Pro Microphone.")
        meeting.micSwitch = .init(from: "AirPods Pro", to: "MacBook Pro Microphone", atLine: meeting.ledger.seen)
        let switched = DropText.for(.meeting, dictation: .idle, meeting: meeting, offer: nil, systemAudioOff: false)
        XCTAssertEqual(switched.detail, "AirPods Pro went. Now recording with MacBook Pro Microphone.")
        XCTAssertEqual(switched.tone, .recording)
        XCTAssertEqual(DropText.switchLine(.init(from: nil, to: "X")), "Your mic went. Now recording with X.")
    }

    func testLivesMicLineSaysWhyThisMic() {
        var meeting = CoreStore.LiveMeeting(record: "r")
        XCTAssertNil(LiveMeetingView.micLine(meeting))
        meeting.micName = "MacBook Pro Microphone"
        meeting.micReason = .defaultInput
        XCTAssertNil(LiveMeetingView.micLine(meeting), "nothing worth saying")
        meeting.micReason = .chosenMissing
        XCTAssertEqual(LiveMeetingView.micLine(meeting), "MacBook Pro Microphone, until your chosen mic is back")
        meeting.micSwitch = .init(from: "AirPods Pro", to: "MacBook Pro Microphone")
        XCTAssertEqual(LiveMeetingView.micLine(meeting), "MacBook Pro Microphone, since AirPods Pro went")
    }
}
