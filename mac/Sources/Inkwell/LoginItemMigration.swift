// Inkwell 0.2's "open at login", carried over to this app's login item (LoginItem), once.
//
// 0.2 opened at login through a launch agent it wrote itself: ~/Library/LaunchAgents/Inkwell.plist,
// label "Inkwell", one program argument, its own executable (<bundle>.app/Contents/MacOS/app),
// run at load. Installed over 0.2, this app has no such executable, so at the next login that
// agent starts nothing and Inkwell stays closed. At launch, when that file is there and names this
// very bundle, the app turns on its own login item (SMAppService; it may then wait for the user's
// approval in System Settings, which the menu-bar item shows) and removes that one file.
//
// Decisions:
// - Only that file is ever read, and it is touched only when it is 0.2's for this bundle: its
//   name, label, single program path and run-at-load all as 0.2 wrote them. A file by that name
//   that is anything else is left alone. So is 0.2's agent for a copy elsewhere (a development
//   build, or 1.0 installed beside 0.2): that one still opens 0.2, and this copy never took its
//   place.
// - Once found, 0.2's agent is removed whether or not the login item could be turned on: it opens
//   an executable this bundle does not have, so it can never start anything again. Either failure
//   is shown to the user (turning it on: do it from the menu-bar item; removing the file: which
//   file to delete) and logged. Never silently.
// - Once: the outcome is recorded in the user's defaults as soon as the question is settled (0.2's
//   agent dealt with, even with a failure; nothing there; not 0.2's), so a user who later turns
//   Open at Login off is never overruled. Not recorded while it cannot be settled here: not an
//   app bundle (`swift run`), 0.2's agent for another copy, or a file that could not be read.
// - Logs name no path: the file lives in the user's home directory.
import AppKit
import Darwin
import os

/// The one launch-agent file the migration reads and may remove.
protocol LaunchAgentFiles {
    /// The file's bytes, or nil when nothing is there. Throws when something is there that is not
    /// a regular file, or cannot be read.
    func read(_ file: URL) throws -> Data?
    func remove(_ file: URL) throws
}

/// This app's login item.
@MainActor
protocol LoginItemRegistrar {
    var state: LoginItem.State { get }
    func turnOn() throws
}

@MainActor
struct LoginItemMigration {
    /// The user's-defaults key that records the migration as settled.
    static let recordKey = "LoginItemCarriedOverFrom02"

    /// Why a step failed. `description` is for the screen only: it may name a file in the user's
    /// home directory, so logs carry the domain and code.
    struct Failure: Error, Equatable, Sendable {
        let domain: String
        let code: Int
        let description: String

        init(domain: String, code: Int, description: String) {
            self.domain = domain
            self.code = code
            self.description = description
        }

        init(_ error: any Error) {
            let error = error as NSError
            self.init(domain: error.domain, code: error.code, description: error.localizedDescription)
        }
    }

    enum Outcome: Equatable {
        /// Settled at an earlier launch: nothing was read.
        case alreadyRecorded
        /// Not running from an app bundle: nothing to register, nothing read.
        case notABundle
        /// No 0.2 agent: 0.2 did not open at login.
        case no02Agent
        /// A file by 0.2's name that is not 0.2's agent: left alone.
        case notInkwell02
        /// 0.2's agent, for a copy of the app other than this one: left alone.
        case anotherCopy
        /// Something is there but could not be read: left alone.
        case unreadable(Failure)
        /// 0.2's agent for this bundle was there. `turnedOn`: the login item's state after turning
        /// it on (on, or waiting for approval), or why that failed. `notRemoved`: why 0.2's agent
        /// could not be removed; nil when it is gone.
        case carriedOver(turnedOn: Result<LoginItem.State, Failure>, notRemoved: Failure?)
    }

    /// 0.2's agent: ~/Library/LaunchAgents/Inkwell.plist.
    let agent: URL
    /// This app's bundle.
    let bundle: URL
    let files: any LaunchAgentFiles
    let loginItem: any LoginItemRegistrar
    let defaults: UserDefaults

    func run() -> Outcome {
        if defaults.bool(forKey: Self.recordKey) { return .alreadyRecorded }
        if loginItem.state == .unavailable { return .notABundle }

        let data: Data?
        do {
            data = try files.read(agent)
        } catch {
            return .unreadable(Failure(error))
        }
        guard let data else { return settled(.no02Agent) }
        guard let program = Self.inkwell02Program(data) else { return settled(.notInkwell02) }
        guard program == Self.canonicalPath(bundle) + "/Contents/MacOS/app" else { return .anotherCopy }

        var failure: Failure?
        do {
            try loginItem.turnOn()
        } catch {
            // Checked below: a login item the user has to approve can report an error and still be
            // registered, waiting.
            failure = Failure(error)
        }
        let state = loginItem.state
        let turnedOn: Result<LoginItem.State, Failure> = state == .on || state == .needsApproval
            ? .success(state)
            : .failure(failure ?? Failure(domain: "Inkwell", code: 0, description: "macOS left Open at Login off."))
        var notRemoved: Failure?
        do {
            try files.remove(agent)
        } catch {
            notRemoved = Failure(error)
        }
        return settled(.carriedOver(turnedOn: turnedOn, notRemoved: notRemoved))
    }

    private func settled(_ outcome: Outcome) -> Outcome {
        defaults.set(true, forKey: Self.recordKey)
        return outcome
    }

    /// The program 0.2's agent opens, when `data` is that agent as 0.2 wrote it: label "Inkwell",
    /// one absolute program path ending in `.app/Contents/MacOS/app`, and run at load.
    static func inkwell02Program(_ data: Data) -> String? {
        guard let plist = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
              plist["Label"] as? String == "Inkwell",
              plist["RunAtLoad"] as? Bool == true,
              let arguments = plist["ProgramArguments"] as? [String], arguments.count == 1,
              let program = arguments.first,
              program.hasPrefix("/"), program.hasSuffix(".app/Contents/MacOS/app")
        else { return nil }
        return program
    }

    /// `url`'s path with every symbolic link resolved, as 0.2 wrote its own (realpath); the path as
    /// given when it cannot be resolved.
    static func canonicalPath(_ url: URL) -> String {
        guard let resolved = realpath(url.path, nil) else { return url.standardizedFileURL.path }
        defer { free(resolved) }
        return String(cString: resolved)
    }

    /// What the user is told, for the outcomes that need them; nil for the rest.
    static func notice(for outcome: Outcome) -> (title: String, detail: String)? {
        guard case .carriedOver(let turnedOn, let notRemoved) = outcome else { return nil }
        let leftOver = notRemoved.map {
            "Inkwell 0.2\u{2019}s old item, ~/Library/LaunchAgents/Inkwell.plist, is still there: "
                + "\($0.description) You can delete that file in the Finder."
        }
        switch turnedOn {
        case .failure(let failure):
            let detail = "Inkwell 0.2 opened at login. \(failure.description) "
                + "You can turn it on from Inkwell\u{2019}s menu-bar item."
            return ("Inkwell could not turn on Open at Login.",
                    [detail, leftOver].compactMap { $0 }.joined(separator: " "))
        case .success(let state):
            guard let leftOver else { return nil }
            let now = state == .on
                ? "Open at Login is on."
                : "Open at Login is waiting for your approval in System Settings > General > Login Items."
            return ("Inkwell could not remove Inkwell 0.2\u{2019}s login item.", "\(now) \(leftOver)")
        }
    }
}

// MARK: - At launch

extension LoginItemMigration {
    /// Runs the migration on this Mac's file system, login item and defaults; logs the outcome and
    /// shows a failure.
    static func runAtLaunch() {
        let migration = LoginItemMigration(
            agent: FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent("Library/LaunchAgents/Inkwell.plist", isDirectory: false),
            bundle: Bundle.main.bundleURL,
            files: SystemLaunchAgentFiles(),
            loginItem: SystemLoginItem(),
            defaults: .standard)
        let outcome = migration.run()
        log(outcome)
        if let notice = notice(for: outcome) {
            let alert = NSAlert()
            alert.messageText = notice.title
            alert.informativeText = notice.detail
            alert.runModal()
        }
    }

    private static func log(_ outcome: Outcome) {
        let log = Logger(subsystem: "com.inkwell.app", category: "login")
        switch outcome {
        case .alreadyRecorded, .notABundle:
            break
        case .no02Agent:
            log.notice("no Inkwell 0.2 login agent: nothing to carry over")
        case .notInkwell02:
            log.notice("a launch agent named as Inkwell 0.2's is not 0.2's: left alone")
        case .anotherCopy:
            log.notice("Inkwell 0.2's login agent opens a copy of the app other than this one: left alone")
        case .unreadable(let failure):
            log.error("the launch agent named as Inkwell 0.2's could not be read (\(failure.domain, privacy: .public) \(failure.code, privacy: .public)): left alone")
        case .carriedOver(let turnedOn, let notRemoved):
            switch turnedOn {
            case .success(let state):
                log.notice("Inkwell 0.2's Open at Login carried over (\(String(describing: state), privacy: .public))")
            case .failure(let failure):
                log.error("Inkwell 0.2 opened at login; Open at Login could not be turned on (\(failure.domain, privacy: .public) \(failure.code, privacy: .public))")
            }
            if let failure = notRemoved {
                log.error("Inkwell 0.2's login agent could not be removed (\(failure.domain, privacy: .public) \(failure.code, privacy: .public))")
            } else {
                log.notice("Inkwell 0.2's login agent removed")
            }
        }
    }
}

/// The real file system. Reads without following a symbolic link: 0.2 wrote a regular file.
struct SystemLaunchAgentFiles: LaunchAgentFiles {
    func read(_ file: URL) throws -> Data? {
        var info = stat()
        guard lstat(file.path, &info) == 0 else {
            let error = errno
            if error == ENOENT { return nil }
            throw POSIXError(POSIXErrorCode(rawValue: error) ?? .EIO)
        }
        guard info.st_mode & S_IFMT == S_IFREG else { throw POSIXError(.EFTYPE) }
        return try Data(contentsOf: file)
    }

    func remove(_ file: URL) throws {
        try FileManager.default.removeItem(at: file)
    }
}

/// The real login item (SMAppService, through LoginItem).
struct SystemLoginItem: LoginItemRegistrar {
    var state: LoginItem.State { LoginItem.state }
    func turnOn() throws { try LoginItem.set(true) }
}
