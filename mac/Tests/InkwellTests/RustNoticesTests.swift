// The notices of the Rust crates linked into the core (Generated/RustNotices.swift): About lists
// every one, and the list is the one generated from this checkout's Cargo.lock. The list itself
// comes from cargo's resolution of the release build (`cargo run -p ink-ffi --bin ink-notices`);
// `ink-notices --check` (mac/scripts/rust-notices.sh) compares the whole file with a fresh run.
import Foundation
import XCTest

@testable import Inkwell

final class RustNoticesTests: XCTestCase {
    /// The repository: this file is mac/Tests/InkwellTests/, four levels down.
    private var root: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    }

    /// 64-bit FNV-1a, as ink-notices computes the fingerprint (the constants are the published ones).
    private func fnv1a(_ bytes: some Sequence<UInt8>) -> String {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in bytes {
            hash = (hash ^ UInt64(byte)) &* 0x0000_0100_0000_01b3
        }
        return String(format: "%016llx", hash)
    }

    func testFnv1aMatchesThePublishedVectors() {
        XCTAssertEqual(fnv1a("".utf8), "cbf29ce484222325")
        XCTAssertEqual(fnv1a("a".utf8), "af63dc4c8601ec8c")
        XCTAssertEqual(fnv1a("foobar".utf8), "85944171f73967e8")
    }

    /// Every crate of the release graph has its row in About, with its licence text.
    func testAboutListsEveryRustCrateTheReleaseLinksWithItsLicenceText() {
        let crates = RustNotices.crates
        XCTAssertGreaterThan(crates.count, 100, "the release links over a hundred crates")
        XCTAssertEqual(RustNotices.heading, "Rust libraries (\(crates.count))")
        XCTAssertEqual(Set(crates.map(\.id)).count, crates.count, "each crate once")
        XCTAssertEqual(crates.map(\.name), crates.map(\.name).sorted(), "in name order")
        for notice in crates {
            XCTAssertFalse(notice.name.hasPrefix("ink-"), "\(notice.id): the workspace's own crates are Inkwell's")
            XCTAssertTrue(notice.text.hasPrefix("--- "), "\(notice.id): its text starts with a file's name")
            // A licence text, not only a statement naming one.
            let texts = [
                "Permission is hereby granted, free of charge",  // MIT
                "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION",  // Apache-2.0
                "Redistribution and use in source and binary forms",  // BSD
                "provided 'as-is', without any express or implied warranty",  // Zlib
            ]
            XCTAssertTrue(texts.contains { notice.text.contains($0) }, "\(notice.id) ships no licence text")
            if notice.shown != notice.licence {
                XCTAssertEqual(notice.detail, "\(notice.licence); used under \(notice.shown)")
            }
        }
    }

    /// The list was generated from this checkout's Cargo.lock and the release's features: a
    /// dependency change without regenerating fails here. (Its crates are cargo's resolution of
    /// the release build; here, each one is at least in the lock at the version listed.)
    func testTheListWasGeneratedFromTheCurrentLockAndTheReleaseFeatures() throws {
        let lock = try String(contentsOf: root.appendingPathComponent("core/Cargo.lock"), encoding: .utf8)
            .replacingOccurrences(of: "\r\n", with: "\n")
        let input = "\(RustNotices.features)\n\(RustNotices.target)\n\(lock)"
        XCTAssertEqual(
            fnv1a(input.utf8), RustNotices.lockFingerprint,
            "Generated/RustNotices.swift was made from another core/Cargo.lock: run `cargo run -p ink-ffi --bin ink-notices`")

        var locked = Set<String>()
        var name: String?
        for line in lock.split(separator: "\n") {
            if line.hasPrefix("name = \"") {
                name = String(line.dropFirst(8).dropLast())
            } else if line.hasPrefix("version = \""), let n = name {
                locked.insert("\(n) \(line.dropFirst(11).dropLast())")
                name = nil
            }
        }
        XCTAssertGreaterThan(locked.count, 100, "the lock was read")
        for notice in RustNotices.crates {
            XCTAssertTrue(locked.contains(notice.id), "\(notice.id) is not in Cargo.lock")
        }

        let script = try String(contentsOf: root.appendingPathComponent("mac/scripts/build-mac.sh"), encoding: .utf8)
        XCTAssertTrue(
            script.contains("release_features=\"\(RustNotices.features)\""),
            "the list is for the features build-mac.sh --engines builds")
        XCTAssertEqual(RustNotices.target, "aarch64-apple-darwin")
    }
}
