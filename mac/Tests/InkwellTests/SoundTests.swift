// Settings > Sound: the microphone picker (Automatic first, then each mic with how it connects, and
// a chosen mic that isn't connected), its captions, and the mic test with its meter.
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

@MainActor
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
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

@MainActor
final class SoundModelTests: XCTestCase {
    func testThePickerListsAutomaticThenEachMicWithHowItConnects() {
        let sent = Sent()
        let sound = SoundModel(send: sent.send)
        sound.load()
        XCTAssertEqual(sent.commands, [.audioDevices(ref: SoundModel.devicesID)])
        XCTAssertEqual(sound.choices, [], "nothing until the core answers")
        XCTAssertEqual(sound.caption, "Reading the microphones…")
        sound.apply(devices())
        XCTAssertEqual(sound.choices.map(\.title), [
            "Automatic (MacBook Pro Microphone)",
            "MacBook Pro Microphone · Built-in",
            "AirPods Pro · Bluetooth",
            "Loopback",
        ])
        XCTAssertEqual(sound.choices.map(\.id), ["auto", "mbp", "pods", "odd"])
        XCTAssertTrue(sound.caption.hasPrefix("Automatic follows the Mac's input"), sound.caption)
        XCTAssertNil(sound.missingLine)
    }

    func testChoosingAMicSendsTheSettingAndARefusalSaysSoAndReadsAgain() {
        let sent = Sent()
        let sound = SoundModel(send: sent.send)
        sound.apply(devices())
        sound.choose("auto")
        XCTAssertEqual(sent.commands, [], "already the choice")
        sound.choose("pods")
        XCTAssertEqual(sent.commands, [.settingSet(.audioInput, "pods")])
        XCTAssertEqual(sound.devices?.input, "pods", "shown at once")
        sound.apply(event(#"{"type":"command.failed","command":"setting.set","id":"setting:audio.input","message":"no microphone with that id is connected"}"#))
        XCTAssertEqual(sound.problem, "Couldn't choose that microphone: no microphone with that id is connected")
        XCTAssertEqual(sent.commands.last, .audioDevices(ref: SoundModel.devicesID), "the choice as it really is")
        sound.apply(devices())
        XCTAssertNil(sound.problem, "a fresh answer clears it")
    }

    func testAChosenBluetoothMicSaysWhatItCosts() {
        let sound = SoundModel(send: { _ in })
        sound.apply(devices(
            input: "pods", using: #"{"id":"pods","name":"AirPods Pro","transport":"bluetooth","reason":"chosen"}"#,
            wanted: #"{"id":"pods","name":"AirPods Pro","transport":"bluetooth"}"#))
        XCTAssertTrue(sound.caption.contains("call quality"), sound.caption)
        sound.apply(devices(
            input: "mbp", using: #"{"id":"mbp","name":"MacBook Pro Microphone","transport":"built_in","reason":"chosen"}"#,
            wanted: #"{"id":"mbp","name":"MacBook Pro Microphone","transport":"built_in"}"#, type: "audio.devices_changed"))
        XCTAssertTrue(sound.caption.hasPrefix("Dictation and meetings both use it"), sound.caption)
    }

    /// The chosen mic isn't connected: it stays in the picker (as the choice), and the caption
    /// says who stands in until it is back.
    func testAChosenMicThatIsNotConnectedStaysChosenAndTheCaptionSaysWhoStandsIn() {
        let sound = SoundModel(send: { _ in })
        sound.apply(devices(
            input: "gone", using: #"{"id":"mbp","name":"MacBook Pro Microphone","transport":"built_in","reason":"chosen_missing"}"#,
            wanted: #"{"id":"gone","name":"Studio Mic","transport":"usb"}"#, type: "audio.devices_changed"))
        XCTAssertEqual(sound.choices.last, SoundModel.Choice(id: "gone", title: "Studio Mic · not connected"))
        XCTAssertEqual(sound.missingLine, "Studio Mic isn't connected. Inkwell is using MacBook Pro Microphone until it is.")
        XCTAssertEqual(sound.caption, sound.missingLine)
    }

    func testNoMicrophoneSaysSo() {
        let sound = SoundModel(send: { _ in })
        sound.apply(event(#"{"type":"audio.devices","input":"auto","inputs":[]}"#))
        XCTAssertEqual(sound.choices.map(\.title), ["Automatic"])
        XCTAssertEqual(sound.caption, "No microphone is connected.")
    }

    /// Test opens the mic; the level follows the core's reports, only while the test runs; Stop
    /// asks the core; the end says whether it heard anything.
    func testTheTestRunsItsLevelAndSaysWhetherItHeardYou() {
        let sent = Sent()
        let sound = SoundModel(send: sent.send)
        sound.apply(devices())
        XCTAssertEqual(sound.testLine, "Speak for a few seconds to see the level. Nothing is kept.")
        sound.toggleTest()
        XCTAssertEqual(sent.commands.last, .audioTest(ref: SoundModel.testID))
        XCTAssertTrue(sound.isTesting)
        XCTAssertEqual(sound.test, .starting)
        sound.apply(event(#"{"type":"audio.test_level","level":0.9}"#))
        XCTAssertEqual(sound.level, 0, "no level before the test has started")
        sound.apply(event(#"{"type":"audio.test_started","ref":"sound.test","mic_name":"MacBook Pro Microphone","mic_transport":"built_in","mic_reason":"default_input","seconds":15}"#))
        XCTAssertEqual(sound.testLine, "Listening with MacBook Pro Microphone…")
        sound.apply(event(#"{"type":"audio.test_level","ref":"sound.test","level":0.42}"#))
        XCTAssertEqual(sound.level, 0.42)
        sound.apply(event(#"{"type":"audio.test_level","ref":"sound.test","level":1.7}"#))
        XCTAssertEqual(sound.level, 1, "clamped to the bar")
        sound.toggleTest()
        XCTAssertEqual(sent.commands.last, .audioTestStop(ref: SoundModel.stopID))
        sound.apply(event(#"{"type":"audio.tested","ref":"sound.test","ended":"stopped","heard":true,"peak":0.8}"#))
        XCTAssertFalse(sound.isTesting)
        XCTAssertEqual(sound.level, 0, "the bar is empty, and still, once it ends")
        XCTAssertEqual(sound.testLine, "Inkwell heard you.")
        XCTAssertFalse(sound.testLineIsProblem)
        sound.toggleTest()
        sound.apply(event(#"{"type":"audio.tested","ref":"sound.test","ended":"done","heard":false,"peak":0.01}"#))
        XCTAssertTrue(sound.testLine.hasPrefix("Not hearing you?"), sound.testLine)
        XCTAssertTrue(sound.testLineIsProblem)
        sound.toggleTest()
        sound.apply(event(#"{"type":"audio.tested","ref":"sound.test","ended":"failed","heard":false,"peak":0,"message":"the microphone AirPods Pro went away"}"#))
        XCTAssertEqual(sound.testLine, "The test stopped: the microphone AirPods Pro went away.")
        sound.toggleTest()
        sound.apply(event(#"{"type":"audio.tested","ref":"sound.test","ended":"meeting","heard":true,"peak":0.5}"#))
        XCTAssertEqual(sound.testLine, "Stopped: a meeting started recording.")
    }

    func testATestRefusedWhileAMeetingRecordsSaysItWaits() {
        let sound = SoundModel(send: { _ in })
        sound.toggleTest()
        sound.apply(event(#"{"type":"command.failed","command":"audio.test","id":"sound.test","code":"meeting_recording","message":"a meeting is recording"}"#))
        XCTAssertFalse(sound.isTesting)
        XCTAssertEqual(sound.testLine, "The test waits until the meeting ends.")
        XCTAssertTrue(sound.testLineIsProblem)
        XCTAssertTrue(sound.handles(failed("audio.test", id: "sound.test")))
        XCTAssertFalse(sound.handles(failed("audio.test", id: "other")))
        sound.toggleTest()
        XCTAssertNotEqual(sound.testLine, "The test waits until the meeting ends.", "a new try clears it")
    }

    private func failed(_ command: String, id: String?) -> CommandFailed {
        guard case .commandFailed(let f) = event(#"{"type":"command.failed","command":"\#(command)"\#(id.map { #","id":"\#($0)""# } ?? ""),"message":"x"}"#) else {
            fatalError("not a command.failed")
        }
        return f
    }

    func testTheCommandsAreTheJSONTheCoreReads() throws {
        func fields(_ c: CoreCommand) throws -> [String: String] {
            try XCTUnwrap(JSONSerialization.jsonObject(with: Data(c.json.utf8)) as? [String: String])
        }
        XCTAssertEqual(try fields(.audioDevices(ref: "a")), ["cmd": "audio.devices", "id": "a"])
        XCTAssertEqual(try fields(.audioTest(ref: "t")), ["cmd": "audio.test", "id": "t"])
        XCTAssertEqual(try fields(.audioTestStop(ref: "s")), ["cmd": "audio.test_stop", "id": "s"])
        XCTAssertEqual(try fields(.settingSet(.audioInput, "auto"))["key"], "audio.input")
        XCTAssertEqual(CoreCommand.audioTestStop(ref: "s").name, "audio.test_stop")
    }

    /// The screens route the events and the failures to the model.
    func testTheScreensRouteSoundToTheModel() {
        let screens = ScreenModels(send: { _ in }, calendar: SoundFakeCalendar())
        screens.apply([devices()])
        XCTAssertEqual(screens.sound.devices?.input, "auto")
        XCTAssertTrue(screens.handles(failed("audio.devices", id: SoundModel.devicesID)))
        XCTAssertTrue(screens.handles(failed("setting.set", id: SoundModel.settingID)))
    }
}

private struct SoundFakeCalendar: CalendarAccess {
    func state() -> CardState { .notAsked }
    func request(done: @escaping @MainActor @Sendable () -> Void) {}
}
