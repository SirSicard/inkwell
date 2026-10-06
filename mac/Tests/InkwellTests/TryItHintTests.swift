// The first run's try-it hint: when the orb heard nothing, where to pick the microphone.
import InkBridge
import XCTest

@testable import Inkwell

final class TryItHintTests: XCTestCase {
    func testAHeldTakeThatHeardNoWordsAndATakeThatFoundNoSpeechShowTheHint() {
        let held = CoreStore.LiveDictation(take: 1, edit: false, mode: nil, app: nil)
        XCTAssertTrue(TryItHint.afterHold(phase: .listening, live: held, hasPartials: true))
        XCTAssertFalse(TryItHint.afterHold(phase: .listening, live: held, hasPartials: false), "no live words to wait for")
        XCTAssertFalse(TryItHint.afterHold(phase: .transcribing, live: held, hasPartials: true), "let go already")
        let speaking = CoreStore.LiveDictation(take: 1, edit: false, mode: nil, app: nil, partial: "hi")
        XCTAssertFalse(TryItHint.afterHold(phase: .listening, live: speaking, hasPartials: true))
        XCTAssertEqual(TryItHint.after(.discarded(.silence)), true)
        XCTAssertEqual(TryItHint.after(.discarded(.noSpeech)), true)
        XCTAssertEqual(TryItHint.after(.discarded(.nothingHeard)), true)
        XCTAssertTrue(TryItHint.heard("so"), "words after a pause hide it")
        XCTAssertFalse(TryItHint.heard("  "))
        XCTAssertFalse(TryItHint.heard(nil))
        XCTAssertEqual(TryItHint.after(.discarded(.tooShort)), nil, "a tap says nothing about the mic")
        XCTAssertEqual(TryItHint.after(.inserted(.pasted)), false, "it heard you")
        XCTAssertTrue(TryItHint.text.contains("Settings > Sound"))
    }
}
