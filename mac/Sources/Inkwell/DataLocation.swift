// Where the app keeps its data: ~/Library/Application Support/Inkwell/ (the core's library,
// recordings and models, and the single-instance lock).
//
// INK_DATA_DIR and INK_MODELS_DIR move the library and the models elsewhere, for development and
// for scripts/idle-budget.sh, which must not touch the user's library. Only whoever launches the
// process can set them, so they grant nothing that process could not already do.
import Foundation

enum DataLocation {
    /// A setting that could not be used.
    struct Invalid: Error, CustomStringConvertible {
        let description: String
    }

    /// ~/Library/Application Support/Inkwell.
    static func defaultDataDirectory() -> URL {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/Application Support", isDirectory: true)
        return support.appendingPathComponent("Inkwell", isDirectory: true)
    }

    /// The library's directory: INK_DATA_DIR when set, else the default.
    static func dataDirectory(environment: [String: String]) throws -> URL {
        try override("INK_DATA_DIR", environment) ?? defaultDataDirectory()
    }

    /// The models' directory: INK_MODELS_DIR when set, else nil (the core uses `<data>/models`).
    static func modelsDirectory(environment: [String: String]) throws -> URL? {
        try override("INK_MODELS_DIR", environment)
    }

    /// Whether INK_DATA_DIR moves the library. A moved library never looks at the user's Inkwell
    /// 0.2 data either (Import02Model.looks).
    static func isMoved(environment: [String: String]) -> Bool {
        !(environment["INK_DATA_DIR"] ?? "").isEmpty
    }

    /// The single-instance lock. Always in the default directory, even when the library is moved:
    /// two running copies would both hold the dictation key and the microphone, whichever library
    /// each one writes to.
    static func instanceLockFile() -> URL {
        defaultDataDirectory().appendingPathComponent("inkwell.lock", isDirectory: false)
    }

    /// Where a second copy asks the running one to show its window (ShowWindowChannel). Next to
    /// the lock, for the same reason.
    static func instanceSocketFile() -> URL {
        defaultDataDirectory().appendingPathComponent("inkwell.sock", isDirectory: false)
    }

    /// Creates `directory` (and its parents) if missing, readable by this user only.
    static func create(_ directory: URL) throws {
        try FileManager.default.createDirectory(
            at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    }

    private static func override(_ name: String, _ environment: [String: String]) throws -> URL? {
        guard let value = environment[name], !value.isEmpty else { return nil }
        // The core refuses a relative path too; saying so here names the variable.
        guard value.hasPrefix("/") else {
            throw Invalid(description: "\(name) must be an absolute path")
        }
        return URL(fileURLWithPath: value, isDirectory: true).standardizedFileURL
    }
}
