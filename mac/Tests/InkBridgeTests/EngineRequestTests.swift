// Reading an engine request from the core: refused when unreadable, never guessed.
import Foundation
@testable import InkBridge
import XCTest

final class EngineRequestTests: XCTestCase {
    private func request(_ options: String?, samples: [Float] = [0.1, 0.2]) -> Result<EngineRequest, InkEngineError> {
        samples.withUnsafeBufferPointer { buffer in
            if let options {
                return options.withCString {
                    EngineRequest.read(samples: buffer.baseAddress, count: buffer.count, options: $0)
                }
            }
            return EngineRequest.read(samples: buffer.baseAddress, count: buffer.count, options: nil)
        }
    }

    func testAWellFormedRequestIsRead() throws {
        let read = try request(#"{"channel":"far","context":"Ink"}"#).get()
        XCTAssertEqual(read.channel, .far)
        XCTAssertEqual(read.context, "Ink")
        XCTAssertEqual(read.samples, [0.1, 0.2])
    }

    /// The channel decides "you" versus "them": a request that does not say it is refused, never
    /// guessed.
    func testAnUnreadableRequestIsRefusedNotDefaulted() {
        for options in [nil, "not json", #"{"context":"x"}"#, #"{"channel":"left"}"#] {
            XCTAssertEqual(request(options), .failure(.badRequest), options ?? "nil")
        }
        let missingSamples = #"{"channel":"mic"}"#.withCString {
            EngineRequest.read(samples: nil, count: 4, options: $0)
        }
        XCTAssertEqual(missingSamples, .failure(.badRequest))
    }
}
