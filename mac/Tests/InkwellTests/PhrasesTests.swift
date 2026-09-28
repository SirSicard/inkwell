// Settings > Snippets and Voice commands, and the note about Inkwell 0.2's dictation key: what each
// model shows for the core's events and what it sends, a voice command heard and not carried out,
// and the commands against the real core. The views are checked by hand
// (mac/SCREENS-B-CHECKLIST.md, section 5b).
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

@MainActor
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }

    var lastSnippets: [SnippetDraft]? {
        for command in commands.reversed() {
            if case .snippetsSave(let rows, _) = command { return rows }
        }
        return nil
    }

    var lastCommands: (enabled: Bool, wake: String, rows: [VoiceCommandDraft])? {
        for command in commands.reversed() {
            if case .voiceCommandsSave(let enabled, let wake, let rows, _) = command { return (enabled, wake, rows) }
        }
        return nil
    }
}

private let importedSnippets = #"{"type":"snippets.listed","from_import":true,"ref":"snippets:1","snippets":[{"id":"s1","trigger":"my sig","expansion":"Kind regards","category":"Email","enabled":true},{"id":"s2","trigger":"brb","expansion":"be right back","category":"","enabled":false}]}"#

@MainActor
final class SnippetsModelTests: XCTestCase {
    func testTheListIsTheCoresAndAnImportSaysSo() {
        let sent = Sent()
        let model = SnippetsModel(send: sent.send)
        model.load()
        guard case .snippetsList = sent.commands.first else { return XCTFail("\(sent.commands)") }
        XCTAssertFalse(model.loaded)
        model.apply(event(importedSnippets))
        XCTAssertTrue(model.loaded)
        XCTAssertTrue(model.fromImport)
        XCTAssertEqual(model.rows.map(\.trigger), ["my sig", "brb"])
        XCTAssertEqual(model.rows[1].enabled, false)
    }

    func testAddEditToggleAndDeleteEachSendTheWholeList() {
        let sent = Sent()
        let model = SnippetsModel(send: sent.send)
        model.apply(event(importedSnippets))

        model.add(trigger: "  ", expansion: "nothing", category: "")
        XCTAssertNil(sent.lastSnippets, "a blank trigger adds nothing")

        model.add(trigger: " addr ", expansion: "1 Example Street", category: "Home")
        var saved = sent.lastSnippets ?? []
        XCTAssertEqual(saved.count, 3)
        XCTAssertEqual(saved.last?.trigger, "addr")
        XCTAssertEqual(saved.last?.category, "Home")
        XCTAssertEqual(Set(saved.map(\.id)).count, 3, "a new id")

        var edited = model.rows[0]
        edited.expansion = "Best wishes"
        model.update(edited)
        XCTAssertEqual(sent.lastSnippets?.first?.expansion, "Best wishes")

        model.setEnabled("s2", true)
        XCTAssertEqual(sent.lastSnippets?[1].enabled, true)

        model.delete("s1")
        saved = sent.lastSnippets ?? []
        XCTAssertEqual(saved.map(\.id).contains("s1"), false)
        XCTAssertEqual(model.rows.count, 2, "shown at once")
    }

    /// Two quick changes: the answer to the first never shows over the second.
    func testOnlyTheAnswerToTheNewestSaveReplacesTheRows() {
        let sent = Sent()
        let model = SnippetsModel(send: sent.send)
        model.load()
        model.apply(event(importedSnippets))
        model.delete("s1")
        model.delete("s2")
        XCTAssertEqual(model.rows, [])
        // The first delete's answer (snippets:2) arrives after the second was sent.
        model.apply(event(#"{"type":"snippets.listed","from_import":false,"ref":"snippets:2","snippets":[{"id":"s2","trigger":"brb","expansion":"be right back","category":"","enabled":false}]}"#))
        XCTAssertEqual(model.rows, [], "an earlier answer is not shown")
        model.apply(event(#"{"type":"snippets.listed","from_import":false,"ref":"snippets:3","snippets":[]}"#))
        XCTAssertEqual(model.rows, [])
        XCTAssertFalse(model.fromImport)
    }

    func testAFailedSaveSaysSoAndReadsTheListAgain() {
        let sent = Sent()
        let model = SnippetsModel(send: sent.send)
        model.apply(event(importedSnippets))
        model.delete("s1")
        let before = sent.commands.count
        model.apply(event(#"{"type":"command.failed","command":"snippets.save","id":"snippets:2","message":"store failed"}"#))
        XCTAssertEqual(model.failure, SnippetsModel.saveFailedText)
        guard case .snippetsList(let reload) = sent.commands.dropFirst(before).first else { return XCTFail("\(sent.commands)") }
        model.apply(event(importedSnippets.replacingOccurrences(of: "snippets:1", with: reload)))
        XCTAssertNil(model.failure)
        XCTAssertEqual(model.rows.count, 2, "the saved list, not the unsaved one")
        model.apply(event(#"{"type":"command.failed","command":"snippets.list","message":"the stored snippets cannot be read"}"#))
        XCTAssertEqual(model.failure, SnippetsModel.loadFailedText)
    }

    func testTheSaveCommandCarriesEveryField() throws {
        let json = CoreCommand.snippetsSave(
            [SnippetDraft(id: "a", trigger: "brb", expansion: "be right back", category: "", enabled: false)], ref: "snippets:9"
        ).json
        let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object["cmd"] as? String, "snippets.save")
        XCTAssertEqual(object["id"] as? String, "snippets:9")
        let first = try XCTUnwrap((object["snippets"] as? [[String: Any]])?.first)
        XCTAssertEqual(first["trigger"] as? String, "brb")
        XCTAssertEqual(first["enabled"] as? Bool, false)
    }
}

private let importedCommands = #"{"type":"voice_commands.listed","enabled":true,"wake_prefix":"inkwell","from_import":true,"commands":[{"id":"sig","triggers":["sign off"],"action":"insert_text","value":"Best, A.","enabled":true,"carried_out":true},{"id":"site","triggers":["open the site","site"],"action":"open_url","value":"https://example.com","enabled":true,"carried_out":false}]}"#

@MainActor
final class VoiceCommandsModelTests: XCTestCase {
    func testImportedCommandsAreListedWithWhatThisBuildDoes() {
        let model = VoiceCommandsModel(send: { _ in })
        model.apply(event(importedCommands))
        XCTAssertTrue(model.enabled)
        XCTAssertTrue(model.fromImport)
        XCTAssertEqual(model.rows.map(\.carriedOut), [true, false])
        XCTAssertEqual(VoiceCommandsModel.describe(model.rows[0]), "Type \u{201C}Best, A.\u{201D}")
        XCTAssertEqual(VoiceCommandsModel.describe(model.rows[1]), "Open https://example.com")
    }

    func testChangesSendTheWholeStore() {
        let sent = Sent()
        let model = VoiceCommandsModel(send: sent.send)
        model.apply(event(importedCommands))
        model.setEnabled(false)
        XCTAssertEqual(sent.lastCommands?.enabled, false)
        XCTAssertEqual(sent.lastCommands?.rows.count, 2, "the commands go with the switch")

        model.setWakePrefix("   ")
        XCTAssertEqual(sent.lastCommands?.wake, "inkwell", "a blank wake word changes nothing")
        model.setWakePrefix(" Computer ")
        XCTAssertEqual(sent.lastCommands?.wake, "computer")

        model.setCommandEnabled("site", false)
        XCTAssertEqual(sent.lastCommands?.rows[1].enabled, false)

        model.add(triggers: "Sign Here, , sig", action: .insertText, value: "A. Writer")
        XCTAssertEqual(sent.lastCommands?.rows.last?.triggers, ["sign here", "sig"])
        XCTAssertEqual(sent.lastCommands?.rows.last?.value, "A. Writer")
        let count = sent.commands.count
        model.add(triggers: "x", action: .openUrl, value: "https://example.com")
        model.add(triggers: " , ", action: .insertText, value: "y")
        XCTAssertEqual(sent.commands.count, count, "only kinds this build does, and never without a phrase")

        model.delete("sig")
        XCTAssertEqual(sent.lastCommands?.rows.map(\.id).contains("sig"), false)
    }

    func testTheSaveCommandCarriesTheActionAndItsValue() throws {
        let json = CoreCommand.voiceCommandsSave(
            enabled: true, wakePrefix: "inkwell",
            commands: [VoiceCommandDraft(id: "u", triggers: ["scratch that"], action: .undo, value: nil),
                       VoiceCommandDraft(id: "t", triggers: ["sign off"], action: .insertText, value: "Best")],
            ref: "voice_commands:3"
        ).json
        let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        let commands = try XCTUnwrap(object["commands"] as? [[String: Any]])
        XCTAssertEqual(commands[0]["action"] as? String, "undo")
        XCTAssertNil(commands[0]["value"])
        XCTAssertEqual(commands[1]["value"] as? String, "Best")
        XCTAssertEqual(object["wake_prefix"] as? String, "inkwell")
    }

    func testAFailedSaveSaysSoAndReadsAgain() {
        let sent = Sent()
        let model = VoiceCommandsModel(send: sent.send)
        model.apply(event(importedCommands))
        model.setEnabled(false)
        model.apply(event(#"{"type":"command.failed","command":"voice_commands.save","id":"voice_commands:1","message":"store failed"}"#))
        XCTAssertEqual(model.failure, VoiceCommandsModel.saveFailedText)
        guard case .voiceCommandsList = sent.commands.last else { return XCTFail("\(sent.commands)") }
        guard case .commandFailed(let failed) = event(#"{"type":"command.failed","command":"voice_commands.save","id":"voice_commands:1","message":"x"}"#) else {
            return XCTFail("not a failure")
        }
        XCTAssertTrue(ScreenModels(send: { _ in }).handles(failed), "the screen shows it")
    }
}

@MainActor
final class ImportNoteTests: XCTestCase {
    func testAnUnmappableKeyIsSaidUntilDismissed() {
        let sent = Sent()
        let model = ImportNoteModel(send: sent.send)
        model.load()
        XCTAssertEqual(sent.commands, [.importNotes])
        model.apply(event(#"{"type":"import.notes","key":{"hotkey":"super+shift+space","outcome":"combination","applied":false,"toggle":false}}"#))
        let note = try? XCTUnwrap(model.note)
        XCTAssertEqual(
            note.map { ImportNoteModel.text($0, currentKey: "fn (Globe)") },
            "Inkwell 0.2 started dictation with \u{2318}\u{21E7}Space, a key combination. Inkwell now listens for one key held on its own, so it uses fn (Globe). Pick another under Dictate if you like.")
        model.dismiss()
        XCTAssertNil(model.note)
        XCTAssertEqual(sent.commands.last, .settingSet(.importKeyNote, "dismissed"))
        model.apply(event(#"{"type":"import.notes"}"#))
        XCTAssertNil(model.note, "nothing to say")
    }

    func testAToggleKeyThatCarriedOverSaysItIsNowHeld() throws {
        let model = ImportNoteModel(send: { _ in })
        model.apply(event(#"{"type":"import.notes","key":{"hotkey":"right_opt","outcome":"mapped","key":"right_option","applied":true,"toggle":true}}"#))
        let text = ImportNoteModel.text(try XCTUnwrap(model.note), currentKey: "Right Option")
        XCTAssertTrue(text.hasPrefix("Inkwell 0.2 started and stopped on separate presses."), text)
        XCTAssertTrue(text.contains("hold Right Option"), text)
    }

    func testOldKeysReadAsKeys() {
        XCTAssertEqual(ImportNoteModel.keys("ctrl+space"), "\u{2303}Space")
        XCTAssertEqual(ImportNoteModel.keys("alt+f13"), "\u{2325}F13")
        XCTAssertEqual(ImportNoteModel.keys("right_cmd"), "right \u{2318}")
        XCTAssertEqual(ImportNoteModel.keys("f13"), "F13")
    }
}

@MainActor
final class VoiceCommandNoticeTests: XCTestCase {
    func testACommandNotCarriedOutIsNeverSilent() {
        let store = CoreStore()
        store.apply([event(#"{"type":"dictation.command","action":"undo","risk":"safe","carried_out":false}"#)])
        XCTAssertEqual(store.notices.map(\.kind), [.voiceCommandNotCarriedOut(.undo)])
        XCTAssertNotNil(NeedsYou.describe(.voiceCommandNotCarriedOut(.undo)))
        store.apply([
            event(#"{"type":"dictation.command","action":"insert_text","risk":"safe","value":"x","carried_out":true}"#),
            event(#"{"type":"dictation.command","action":"change_style","risk":"safe","value":"casual"}"#),
        ])
        XCTAssertEqual(store.notices.count, 1, "carried out, or an older core that does not say")
    }
}

/// Against the real core: the commands are read and answered with events this shell decodes.
final class PhrasesCoreContractTests: XCTestCase {
    func testTheListsAndTheNoteAreReadByTheCoreAndAnswered() throws {
        let data = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-phrases-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: data) }
        let events = Mutex<[InkEvent]>([])
        let session = try InkSession.start(InkConfig(dataDir: data.path, logLevel: "warn")) { event in
            events.withLock { $0.append(event) }
        }
        defer { session.shutdown() }
        func answer<T>(_ command: CoreCommand, _ pick: (InkEvent) -> T?) throws -> T? {
            let before = events.withLock { $0.count }
            try session.command(command.json)
            let until = Date().addingTimeInterval(10)
            while Date() < until {
                if let found = events.withLock({ Array($0.dropFirst(before)) }).lazy.compactMap(pick).first {
                    return found
                }
                Thread.sleep(forTimeInterval: 0.01)
            }
            return nil
        }
        let empty = try answer(.snippetsList(ref: "snippets:1")) { if case .snippetsListed(let l) = $0 { l } else { nil } }
        XCTAssertEqual(empty?.snippets, [])
        XCTAssertEqual(empty?.fromImport, false)
        let saved = try answer(.snippetsSave([SnippetDraft(id: "a", trigger: "brb", expansion: "be right back")], ref: "snippets:2")) {
            if case .snippetsListed(let l) = $0, l.ref == "snippets:2" { l } else { nil }
        }
        XCTAssertEqual(saved?.snippets.first?.expansion, "be right back")
        let defaults = try answer(.voiceCommandsList(ref: "voice_commands:1")) { if case .voiceCommandsListed(let l) = $0 { l } else { nil } }
        XCTAssertEqual(defaults?.enabled, false, "off until the user turns them on")
        XCTAssertFalse(defaults?.commands.isEmpty ?? true)
        let commands = try answer(.voiceCommandsSave(
            enabled: true, wakePrefix: "inkwell",
            commands: [VoiceCommandDraft(id: "t", triggers: ["sign off"], action: .insertText, value: "Best")],
            ref: "voice_commands:2")) {
            if case .voiceCommandsListed(let l) = $0, l.ref == "voice_commands:2" { l } else { nil }
        }
        XCTAssertEqual(commands?.commands.first?.carriedOut, true)
        let notes = try answer(.importNotes) { if case .importNotes(let n) = $0 { n } else { nil } }
        XCTAssertNotNil(notes)
        XCTAssertNil(notes?.key, "no import, nothing to say")
        let dismissed = try answer(.settingSet(.importKeyNote, "dismissed")) { if case .settingValue(let v) = $0 { v } else { nil } }
        XCTAssertEqual(dismissed?.value, "dismissed")
        let undecodable = events.withLock { $0 }.filter { if case .undecodable = $0 { true } else { false } }
        XCTAssertEqual(undecodable, [])
    }
}
