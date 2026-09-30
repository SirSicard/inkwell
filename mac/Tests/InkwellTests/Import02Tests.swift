// Inkwell 0.2's import on the Mac: what the first run's step and Settings > Voice show for the
// core's answers, and what they send. Synthetic events only: no test here starts a core, since a
// real core looks in this Mac's own 0.2 data directory.
import Foundation
import InkBridge
import Synchronization
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

/// What was logged.
private final class Logged: Sendable {
    private let lines = Mutex<[String]>([])
    var messages: [String] { lines.withLock { $0 } }
    var log: ScreenLog { ScreenLog { [self] message in lines.withLock { $0.append(message) } } }
}

/// What a model sent.
@MainActor
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
}

private let counts = #"{"dictations":12,"dictionary_entries":0,"snippets":1,"modes":2,"settings":4,"voice_commands":0,"app_style_rules":0,"linked_keys":0}"#

private func checked(_ state: String, counts: String? = nil, message: String? = nil, ref: String = "import.check") -> InkEvent {
    var fields = [#""type":"import.checked""#, #""state":"\#(state)""#, #""ref":"\#(ref)""#]
    if let counts { fields.append(#""counts":\#(counts)"#) }
    if let message { fields.append(#""message":"\#(message)""#) }
    return event("{" + fields.joined(separator: ",") + "}")
}

@MainActor
final class Import02ModelTests: XCTestCase {
    func testFoundDataIsOfferedWithItsCountsInWords() {
        let sent = Sent()
        let model = Import02Model(send: sent.send)
        XCTAssertFalse(model.offered, "nothing until the core answers")
        model.check()
        XCTAssertEqual(sent.commands, [.importCheck])
        model.apply(checked("found", counts: counts))
        XCTAssertTrue(model.offered)
        XCTAssertTrue(model.canImport)
        XCTAssertEqual(
            model.line,
            "Inkwell 0.2 left 12 dictations, 1 snippet, 2 modes and your settings on this Mac. Import brings them into this library; 0.2\u{2019}s own copy stays as it is.")
    }

    func testNothingToImportIsNotOffered() {
        for state in ["absent", "imported"] {
            let model = Import02Model(send: { _ in })
            model.apply(checked(state))
            XCTAssertFalse(model.offered, state)
            XCTAssertFalse(model.canImport, state)
            XCTAssertEqual(model.line, "", state)
        }
    }

    func testDataThatCannotBeReadNowSaysWhy() {
        let model = Import02Model(send: { _ in })
        model.apply(checked("unreadable", message: "Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again"))
        XCTAssertTrue(model.offered, "Import may work once the reason is gone")
        XCTAssertTrue(model.canImport)
        XCTAssertEqual(
            model.line,
            "Inkwell 0.2\u{2019}s data is on this Mac but can\u{2019}t be read now: Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again.")
    }

    func testAnImportSaysWhatCameOverAndRunsOnce() {
        let sent = Sent()
        let model = Import02Model(send: sent.send)
        model.apply(checked("found", counts: counts))
        model.run()
        XCTAssertEqual(sent.commands, [.importRun])
        XCTAssertFalse(model.canImport, "not twice at once")
        model.run()
        model.check()
        XCTAssertEqual(sent.commands, [.importRun], "no second import, and no look while one runs")
        model.apply(event(#"{"type":"import.finished","counts":{"dictations":12,"dictionary_entries":0,"snippets":1,"modes":2,"settings":4,"voice_commands":0,"app_style_rules":0,"linked_keys":1},"ref":"import.run"}"#))
        XCTAssertTrue(model.offered, "what came over is said where it was pressed")
        XCTAssertFalse(model.canImport)
        XCTAssertEqual(model.line, "Brought over from Inkwell 0.2: 12 dictations, 1 snippet, 2 modes, your settings and 1 saved API key.")
        // A look after it says imported; the report stays for this session.
        model.check()
        model.apply(checked("imported"))
        XCTAssertTrue(model.offered)
        model.run()
        XCTAssertEqual(sent.commands, [.importRun, .importCheck])
    }

    func testAFailedImportIsSaidInWordsAndCanBeTriedAgain() {
        let sent = Sent()
        let model = Import02Model(send: sent.send)
        model.apply(checked("found", counts: counts))
        model.run()
        model.apply(event(#"{"type":"command.failed","command":"import.run","id":"import.run","message":"Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again"}"#))
        XCTAssertEqual(model.failure, "Couldn\u{2019}t import: Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again.")
        XCTAssertTrue(model.canImport)
        model.run()
        XCTAssertEqual(sent.commands, [.importRun, .importRun])
        XCTAssertNil(model.failure, "cleared while it tries again")
    }

    func testALaterLookThatCanReadTheDataClearsAnOldFailure() {
        let sent = Sent()
        let model = Import02Model(send: sent.send)
        model.apply(checked("found", counts: counts))
        model.run()
        model.apply(event(#"{"type":"command.failed","command":"import.run","id":"import.run","message":"Inkwell 0.2 is in the middle of saving its history. Quit Inkwell 0.2, then try again"}"#))
        XCTAssertNotNil(model.failure)
        // 0.2 quit, Settings opened again: the look reads the data now.
        model.check()
        XCTAssertEqual(sent.commands, [.importRun, .importCheck])
        model.apply(checked("found", counts: counts))
        XCTAssertNil(model.failure, "the old failure no longer applies")
        XCTAssertTrue(model.canImport)
        XCTAssertEqual(
            model.line,
            "Inkwell 0.2 left 12 dictations, 1 snippet, 2 modes and your settings on this Mac. Import brings them into this library; 0.2\u{2019}s own copy stays as it is.")
    }

    func testALookThatFailedIsLoggedAndOffersNothing() {
        let logged = Logged()
        let model = Import02Model(send: { _ in }, log: logged.log)
        model.check()
        model.apply(event(#"{"type":"command.failed","command":"import.check","id":"import.check","message":"the library could not be read"}"#))
        XCTAssertTrue(model.checkFailed)
        XCTAssertFalse(model.offered, "the first run goes without the step")
        XCTAssertEqual(model.line, "Couldn\u{2019}t look for Inkwell 0.2\u{2019}s data.")
        XCTAssertEqual(logged.messages.count, 1)
        XCTAssertTrue(logged.messages[0].contains("import.check"), logged.messages[0])
    }

    func testAnswersToOtherCommandsAreNotTheImports() {
        let model = Import02Model(send: { _ in })
        model.apply(checked("found", counts: counts, ref: "someone-else"))
        model.apply(event(#"{"type":"command.failed","command":"import.notes","id":"import.notes","message":"x"}"#))
        XCTAssertNil(model.found)
        XCTAssertFalse(model.checkFailed)
        XCTAssertNil(model.failure)
    }

    func testTheFirstRunLooksOnce() {
        let sent = Sent()
        let model = Import02Model(send: sent.send)
        model.checkOnce()
        model.checkOnce()
        XCTAssertEqual(sent.commands, [.importCheck])
    }

    func testAMovedLibraryNeverLooks() {
        let sent = Sent()
        let model = Import02Model(send: sent.send)
        model.looks = false
        model.checkOnce()
        model.check()
        model.run()
        XCTAssertEqual(sent.commands, [])
        XCTAssertFalse(model.offered)
    }

    func testTheCommandsCarryTheirNameAsTheirId() throws {
        for (command, name) in [(CoreCommand.importCheck, "import.check"), (.importRun, "import.run")] {
            let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(command.json.utf8)) as? [String: String])
            XCTAssertEqual(fields, ["cmd": name, "id": name], "no path: the core knows where to look")
            XCTAssertEqual(command.name, name)
        }
    }
}

@MainActor
final class Import02FirstRunTests: XCTestCase {
    func testTheStepIsShownOnlyWhileThereIsSomethingToImport() {
        let screens = ScreenModels(send: { _ in })
        let onboarding = screens.onboarding
        XCTAssertEqual(onboarding.steps, [.welcome, .permissions, .polish, .ready])
        onboarding.step = .permissions
        onboarding.next()
        XCTAssertEqual(onboarding.step, .polish, "no 0.2 data: no step")

        screens.import02.apply(checked("found", counts: counts))
        XCTAssertEqual(onboarding.steps, [.welcome, .permissions, .importData, .polish, .ready])
        onboarding.back()
        XCTAssertEqual(onboarding.step, .importData)
        onboarding.back()
        XCTAssertEqual(onboarding.step, .permissions)
        onboarding.next()
        XCTAssertEqual(onboarding.step, .importData)
        onboarding.next()
        XCTAssertEqual(onboarding.step, .polish, "Not now moves on")
    }

    func testTheFirstRunLooksForTheDataOnceItShows() {
        let sent = Sent()
        let screens = ScreenModels(send: sent.send)
        screens.apply([event(#"{"type":"setting.value","key":"onboarding.done","value":"true"}"#)])
        XCTAssertFalse(sent.commands.contains(.importCheck), "a first run already done does not look")

        let first = Sent()
        let fresh = ScreenModels(send: first.send)
        fresh.apply([event(#"{"type":"setting.value","key":"onboarding.done"}"#)])
        fresh.apply([event(#"{"type":"setting.value","key":"dictation.polish","value":"off"}"#)])
        XCTAssertEqual(first.commands.filter { $0 == .importCheck }.count, 1)
    }

    func testAnImportReadsTheKeyNoteAndTheKeyAgain() {
        let sent = Sent()
        let screens = ScreenModels(send: sent.send)
        screens.apply([event(#"{"type":"import.finished","counts":\#(counts),"ref":"import.run"}"#)])
        XCTAssertTrue(sent.commands.contains(.importNotes), "the key note appears")
        XCTAssertTrue(sent.commands.contains(.settingGet(.dictationKey)))
        XCTAssertNotNil(screens.import02.imported)
    }

    /// Settings may have read its lists before the import (Import pressed in Settings > Voice): they
    /// are read again, so an edit afterwards keeps what came over instead of saving the old list
    /// over it (the user's own list wins over the import's in the core).
    func testAnImportReadsTheListsSettingsShowsAgain() {
        let sent = Sent()
        let screens = ScreenModels(send: sent.send)
        screens.snippets.load()
        screens.apply([event(#"{"type":"snippets.listed","from_import":false,"ref":"snippets:1","snippets":[]}"#)])
        sent.commands = []

        screens.apply([event(#"{"type":"import.finished","counts":\#(counts),"ref":"import.run"}"#)])
        XCTAssertTrue(sent.commands.contains(.snippetsList(ref: "snippets:2")), "\(sent.commands)")
        XCTAssertTrue(sent.commands.contains(.voiceCommandsList(ref: "voice_commands:1")), "\(sent.commands)")
        XCTAssertTrue(sent.commands.contains(.modesList), "\(sent.commands)")

        screens.apply([event(#"{"type":"snippets.listed","from_import":true,"ref":"snippets:2","snippets":[{"id":"s1","trigger":"my sig","expansion":"Kind regards","category":"","enabled":true}]}"#)])
        XCTAssertTrue(screens.snippets.fromImport)
        screens.snippets.add(trigger: "brb", expansion: "be right back", category: "")
        guard case .snippetsSave(let saved, _, _)? = sent.commands.last else {
            return XCTFail("no save: \(sent.commands)")
        }
        XCTAssertEqual(saved.map(\.trigger), ["my sig", "brb"], "what came over is kept")
    }

    func testTheImportsFailuresAreShownWhereItIsOffered() {
        let screens = ScreenModels(send: { _ in })
        for command in ["import.check", "import.run"] {
            guard case .commandFailed(let failed) = event(#"{"type":"command.failed","command":"\#(command)","id":"\#(command)","message":"x"}"#) else {
                return XCTFail("not a failure")
            }
            XCTAssertTrue(screens.handles(failed), command)
        }
    }
}
