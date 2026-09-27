// The forward-compatibility contract of the generated event types: what a newer or older core
// might send still decodes, keeping what it can.
import Foundation
import InkBridge
import XCTest

final class EventDecodingTests: XCTestCase {
    private func decode(_ json: String) throws -> InkEvent {
        try InkEvent.decode(Data(json.utf8))
    }

    func testAnUnknownEventTypeIsKeptByName() throws {
        XCTAssertEqual(try decode(#"{"type":"future.event","x":1}"#), .unknown(type: "future.event"))
    }

    func testAnUnknownValueInsideAKnownEventKeepsItsTypeAndRecord() throws {
        let event = try decode(
            #"{"type":"meeting.side_state","record":"r1","channel":"mic","state":"sleeping"}"#)
        XCTAssertEqual(event, .undecodable(type: "meeting.side_state", record: "r1"))
        // Without a record, the type is still kept.
        XCTAssertEqual(
            try decode(#"{"type":"dictation.discarded","reason":"bored"}"#),
            .undecodable(type: "dictation.discarded", record: nil))
    }

    func testExtraFieldsAreIgnored() throws {
        let event = try decode(#"{"type":"core.ready","abi":1,"version":"0.0.0","later":{"a":[1]}}"#)
        guard case .coreReady(let ready) = event else {
            return XCTFail("\(event)")
        }
        XCTAssertEqual(ready.abi, 1)
    }

    func testEventsRoundTrip() throws {
        for json in [
            #"{"type":"meeting.side_state","record":"r1","channel":"far","state":"zeros"}"#,
            #"{"type":"future.event"}"#,
            #"{"type":"meeting.side_state","record":"r1","channel":"mic","state":"sleeping"}"#,
        ] {
            let event = try decode(json)
            XCTAssertEqual(try InkEvent.decode(JSONEncoder().encode(event)), event, json)
        }
    }

    func testJSONWithoutATypeStillThrows() {
        XCTAssertThrowsError(try decode(#"{"abi":1}"#))
    }
}
