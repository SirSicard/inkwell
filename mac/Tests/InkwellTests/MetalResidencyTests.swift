// llama.cpp's Metal residency heartbeat is off unless a benchmark keeps it (MetalResidency). Each
// test starts and ends with the variable unset in this process.
import Foundation
import XCTest

@testable import Inkwell

final class MetalResidencyTests: XCTestCase {
    private var ggml: String? { getenv(MetalResidency.ggmlVariable).map { String(cString: $0) } }

    override func setUp() {
        super.setUp()
        unsetenv(MetalResidency.ggmlVariable)
    }

    override func tearDown() {
        unsetenv(MetalResidency.ggmlVariable)
        super.tearDown()
    }

    func testResidencyIsTurnedOffByDefault() {
        XCTAssertTrue(MetalResidency.configure(environment: [:]))
        XCTAssertEqual(ggml, "1")
    }

    func testABenchmarkCanKeepResidency() {
        XCTAssertFalse(MetalResidency.configure(environment: [MetalResidency.keepVariable: "1"]))
        XCTAssertNil(ggml, "the heartbeat stays: the variable is never set")
    }

    func testAnyOtherValueOfTheKeepSwitchStillTurnsItOff() {
        XCTAssertTrue(MetalResidency.configure(environment: [MetalResidency.keepVariable: "0"]))
        XCTAssertEqual(ggml, "1")
    }

    func testAValueAlreadySetIsLeftAlone() {
        setenv(MetalResidency.ggmlVariable, "yes", 1)
        XCTAssertTrue(MetalResidency.configure(environment: [MetalResidency.ggmlVariable: "yes"]))
        XCTAssertEqual(ggml, "yes")
    }
}
