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

    func testTheEchoEventsDecode() throws {
        let found = try decode(
            #"{"type":"meeting.echo","record":"r1","state":"cancelling","from_ms":10000,"unprotected_ms":10000,"stable_from_ms":7000,"delay_ms":-4.5,"drift_ppm":1.63}"#
        )
        guard case .meetingEcho(let echo) = found else {
            return XCTFail("\(found)")
        }
        XCTAssertEqual(echo.state, .cancelling)
        XCTAssertEqual(echo.unprotectedMs, 10000)
        XCTAssertEqual(echo.delayMs, -4.5)

        let failed = try decode(
            #"{"type":"meeting.echo","record":"r1","state":"failed","failure":"backlog","channel":"far"}"#)
        guard case .meetingEcho(let why) = failed else {
            return XCTFail("\(failed)")
        }
        XCTAssertEqual(why.failure, .backlog)
        XCTAssertEqual(why.channel, .far)
        let stalled = try decode(
            #"{"type":"meeting.echo","record":"r1","state":"failed","failure":"stalled"}"#)
        guard case .meetingEcho(let hung) = stalled else {
            return XCTFail("\(stalled)")
        }
        XCTAssertEqual(hung.failure, .stalled)
        let noPath = try decode(
            #"{"type":"meeting.warning","record":"r1","kind":"echo_path_not_found","channel":"mic","phase":"final","audible_ms":12000}"#
        )
        guard case .meetingWarningEvent(let warning) = noPath else {
            return XCTFail("\(noPath)")
        }
        XCTAssertEqual(warning.kind, .echoPathNotFound)

        let pass = try decode(
            #"{"type":"meeting.echo_pass","record":"r1","path":{"delay_ms":46.04,"drift_ppm":1.63,"inliers":64},"windows":81,"candidates":72,"cancelled":true,"erle_db":26,"removed":1,"kept_near_speech":0,"kept_no_evidence":0,"live_echo_finals":2}"#
        )
        guard case .meetingEchoPass(let p) = pass else {
            return XCTFail("\(pass)")
        }
        XCTAssertEqual(p.path?.inliers, 64)
        XCTAssertNil(p.erleFirstDb)
        XCTAssertEqual(p.liveEchoFinals, 2)

        // The removed lines come by place and span: no words.
        let removed = try decode(
            #"{"type":"meeting.removed_as_echo","record":"r1","lines":[{"index":0,"start_ms":19700,"end_ms":20500,"far":[{"start_ms":19550,"end_ms":20550}],"words":6,"matched":6}]}"#
        )
        guard case .meetingRemovedAsEcho(let lines) = removed else {
            return XCTFail("\(removed)")
        }
        XCTAssertEqual(lines.lines.first?.index, 0)
        XCTAssertEqual(lines.lines.first?.far.first?.startMs, 19550)
        for event in [found, failed, pass, removed] {
            XCTAssertEqual(try InkEvent.decode(JSONEncoder().encode(event)), event)
        }
    }

    func testJSONWithoutATypeStillThrows() {
        XCTAssertThrowsError(try decode(#"{"abi":1}"#))
    }
}
