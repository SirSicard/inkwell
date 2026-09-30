// Where Parakeet is loaded from: the directory the core installs its registry row in, only once the
// core has finished installing it, and never by a download of FluidAudio's own. No model runs here:
// the directories are made up in a temp folder.
@testable import AppleEngines
import Foundation
import XCTest

final class ParakeetFilesTests: XCTestCase {
    private var models: URL!

    override func setUpWithError() throws {
        models = FileManager.default.temporaryDirectory
            .appendingPathComponent("inkwell-parakeet-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: models, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: models)
    }

    func testTheModelsAreLookedForWhereTheCoreInstallsTheRow() {
        let root = URL(fileURLWithPath: "/tmp/ink-models", isDirectory: true)
        XCTAssertEqual(
            ParakeetFiles.directory(in: root).path,
            "/tmp/ink-models/parakeet-tdt-0.6b-v3-coreml/7dd20fe6b179/parakeet-tdt-0.6b-v3")
    }

    /// The core writes the full revision into the row's marker once every file checked out; files
    /// without it are an interrupted download, or another revision's.
    func testOnlyAFinishedInstallOfThisRevisionCounts() throws {
        XCTAssertFalse(ParakeetFiles.installed(in: models), "nothing downloaded")
        let row = ParakeetFiles.rowDirectory(in: models)
        try FileManager.default.createDirectory(at: row, withIntermediateDirectories: true)
        let marker = row.appendingPathComponent(".revision")
        try Data((ParakeetFiles.revision + "\n").utf8).write(to: marker)
        XCTAssertFalse(ParakeetFiles.installed(in: models), "not exactly the revision")
        try Data(String(repeating: "0", count: 40).utf8).write(to: marker)
        XCTAssertFalse(ParakeetFiles.installed(in: models), "another revision")
        try Data(ParakeetFiles.revision.utf8).write(to: marker)
        XCTAssertTrue(ParakeetFiles.installed(in: models))
    }

    /// Without a finished install, a load finds no models: nothing is fetched, and nothing in the
    /// models directory is created or removed, even when FluidAudio is handed files it cannot load.
    func testALoadWithoutAFinishedInstallFindsNoModelsAndTouchesNothing() async throws {
        #if !arch(arm64)
            throw XCTSkip("Parakeet runs on Apple silicon only")
        #else
            for directory in [nil, models] {
                await assertLoadFails(from: directory, with: [.modelMissing])
            }
            XCTAssertEqual(try contents(), [], "nothing was created")

            // An interrupted download: the bundles' folders are there, the marker is not.
            let folder = ParakeetFiles.directory(in: models)
            for bundle in ["Preprocessor.mlmodelc", "Encoder.mlmodelc", "Decoder.mlmodelc", "JointDecisionv3.mlmodelc"] {
                try FileManager.default.createDirectory(
                    at: folder.appendingPathComponent(bundle, isDirectory: true), withIntermediateDirectories: true)
            }
            try Data("{}".utf8).write(to: folder.appendingPathComponent("parakeet_vocab.json"))
            let interrupted = try contents()
            await assertLoadFails(from: models, with: [.modelMissing])

            // The same files with the marker reach FluidAudio, which cannot load them: offline, it
            // reports that, and neither fetches nor deletes (a fetch would read `downloadRefused`).
            try Data(ParakeetFiles.revision.utf8).write(
                to: ParakeetFiles.rowDirectory(in: models).appendingPathComponent(".revision"))
            await assertLoadFails(from: models, with: [.modelMissing, .loadFailed(code: 1)])
            XCTAssertEqual(try contents(), (interrupted + ["parakeet-tdt-0.6b-v3-coreml/7dd20fe6b179/.revision"]).sorted())
        #endif
    }

    /// The row's values `ParakeetFiles` restates are the core's (ink-engines): its id, its folder
    /// (FluidAudio's own for v3), the revision it pins, and how its directory and marker are named.
    /// A pin moved in the core alone would have the app look for Parakeet where it is not.
    func testTheRowsLayoutIsTheCoresOwn() throws {
        let engines = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("core/crates/ink-engines/src")
        let rows = try String(contentsOf: engines.appendingPathComponent("rows.rs"), encoding: .utf8)
        let layout = try String(contentsOf: engines.appendingPathComponent("model_dir.rs"), encoding: .utf8)
        XCTAssertEqual(try constant("PARAKEET_COREML_ID", in: rows), ParakeetFiles.rowID)
        XCTAssertEqual(
            try constant("PARAKEET_COREML_FOLDER", in: rows),
            ParakeetFiles.directory(in: models).lastPathComponent)
        let row = try XCTUnwrap(rows.range(of: "pub fn parakeet_tdt_v3_coreml()"), "the row's function")
        XCTAssertEqual(try constant("REVISION", in: String(rows[row.upperBound...])), ParakeetFiles.revision)
        XCTAssertEqual(try constant("REVISION_MARKER", in: layout), ParakeetFiles.marker)
        let length = try XCTUnwrap(layout.firstMatch(of: /const REVISION_DIR_LEN: usize = (\d+);/), "REVISION_DIR_LEN")
        XCTAssertEqual(Int(length.output.1), ParakeetFiles.revisionDirLength)
    }

    /// The first `const <name>: &str = "<value>"` in `source`.
    private func constant(_ name: String, in source: String) throws -> String {
        let pattern = try Regex("const \(name): &str = \"([^\"]*)\"")
        let match = try XCTUnwrap(source.firstMatch(of: pattern), "\(name) is not in the core's source")
        return try XCTUnwrap(match.output[1].substring.map(String.init))
    }

    /// Every file and folder under the models directory, relative to it.
    private func contents() throws -> [String] {
        let base = models.resolvingSymlinksInPath().path + "/"
        let walk = try XCTUnwrap(FileManager.default.enumerator(at: models, includingPropertiesForKeys: nil))
        return try walk.map { item in
            let path = try XCTUnwrap(item as? URL).resolvingSymlinksInPath().path
            XCTAssertTrue(path.hasPrefix(base), path)
            return String(path.dropFirst(base.count))
        }.sorted()
    }

    private func assertLoadFails(
        from directory: URL?, with expected: [ParakeetError], file: StaticString = #filePath, line: UInt = #line
    ) async {
        do {
            _ = try await FluidAudioParakeet.load(from: directory)
            XCTFail("loaded", file: file, line: line)
        } catch {
            XCTAssertTrue(expected.contains(error), "\(error)", file: file, line: line)
        }
    }
}
