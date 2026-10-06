// Settings > Modes' editor: what each row and the editor say for the core's listing (the chips, a
// mode's own model), what each save, delete, confirm and OK sends, every refusal's words, and the
// same against the real core. The views' layout at 720 pt is in ModesLayoutTests.
import AppKit
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
    /// Holds its Sent strongly (a test that drops the tuple's Sent still sends).
    lazy var send: SendCommand = { [self] in self.commands.append($0) }

    /// The modes.save commands' JSON, decoded.
    var saves: [[String: Any]] {
        commands.compactMap { command in
            guard case .modesSave = command else { return nil }
            return try? JSONSerialization.jsonObject(with: Data(command.json.utf8)) as? [String: Any]
        }
    }
}

private struct NoApps: AppDirectory {
    func app(bundleID: String) -> (name: String, icon: NSImage)? { nil }
}

private struct FakeRunning: RunningApps {
    var apps: [PickedApp] = []
    func running() -> [PickedApp] { apps }
}

/// A mode as modes.listed lists it.
private func mode(
    _ id: String, _ name: String, style: String = "casual", polish: Bool = true, fillers: Bool = true,
    prompt: String = "", apps: [String] = [], model: String? = nil, modelName: String? = nil, state: String? = nil
) -> String {
    var fields = #""id":"\#(id)","name":"\#(name)","style":"\#(style)","polish":\#(polish),"remove_fillers":\#(fillers),"polish_prompt":"\#(prompt)","apps":\#(json(apps))"#
    if let model { fields += #","polish_model":"\#(model)""# }
    if let modelName { fields += #","polish_model_name":"\#(modelName)""# }
    if let state { fields += #","polish_model_state":"\#(state)""# }
    return "{\(fields)}"
}

private func json(_ value: Any) -> String {
    String(decoding: (try? JSONSerialization.data(withJSONObject: value)) ?? Data("null".utf8), as: UTF8.self)
}

private let apple = #"{"id":"engine:apple-foundation-models","name":"SystemLanguageModel.default","model":"SystemLanguageModel.default","to":"on_device","allowed":true,"blocked_local_only":false}"#
private func groq(allowed: Bool, blocked: Bool = false) -> String {
    #"{"id":"provider:groq","name":"Groq","model":"llama-3.1-8b-instant","to":"cloud","endpoint":"https://api.groq.com/openai/v1","allowed":\#(allowed),"blocked_local_only":\#(blocked)}"#
}

private func listing(_ modes: [String], models: [String] = [apple], setting: String? = "engine:apple-foundation-models", ref: String? = nil, saved: String? = nil) -> InkEvent {
    var fields = #""type":"modes.listed","default_id":"d","default_polish_prompt":"Tidy it.","modes":[\#(modes.joined(separator: ","))],"polish_models":[\#(models.joined(separator: ","))]"#
    if let setting { fields += #","setting_polish_model":"\#(setting)""# }
    if let ref { fields += #","ref":"\#(ref)""# }
    if let saved { fields += #","saved":"\#(saved)""# }
    return event("{\(fields)}")
}

private let defaultMode = mode("d", "Default", style: "formal", polish: false)

private func polishState(on: Bool, consents: [String] = [], ref: String? = nil) -> InkEvent {
    var fields = #""type":"consent.state","feature":"polish","on":\#(on),"allowed":true,"to":"on_device","name":"SystemLanguageModel.default","consents":[\#(consents.joined(separator: ","))]"#
    if let ref { fields += #","ref":"\#(ref)""# }
    return event("{\(fields)}")
}

private let groqConsent = #"{"to":"cloud","name":"Groq","endpoint":"https://api.groq.com/openai/v1"}"#
private let deviceConsent = #"{"to":"on_device"}"#

private func failed(_ command: String, id: String, code: String?) -> InkEvent {
    let code = code.map { #","code":"\#($0)""# } ?? ""
    return event(#"{"type":"command.failed","command":"\#(command)","id":"\#(id)","message":"refused: the mode's name zebra"\#(code)}"#)
}

@MainActor
final class ModesRowsTests: XCTestCase {
    private func model(switchOn: Bool? = true) -> (ModesModel, Sent) {
        let sent = Sent()
        let modes = ModesModel(send: sent.send, apps: NoApps(), polishSwitch: { switchOn })
        return (modes, sent)
    }

    /// The "Polish" chip shows only when polish runs; with no model at all it does not show, and with
    /// one that can't be used now it is "Polish · off", saying why.
    func testThePolishChipShowsOnlyWithAModel() {
        let (none, _) = model()
        none.apply(listing([mode("c", "Chat"), defaultMode], models: [], setting: nil))
        XCTAssertEqual(none.rows[0].polish, .noModel)
        XCTAssertNil(none.rows[0].polish.chip, "no model: no chip")
        XCTAssertNil(none.rows[0].polish.rowNote)

        let (ready, _) = model()
        ready.apply(listing([mode("c", "Chat"), mode("q", "Quiet", polish: false), defaultMode]))
        XCTAssertEqual(ready.rows[0].polish.chip, "Polish")
        XCTAssertFalse(ready.rows[0].polish.chipDimmed)
        XCTAssertNil(ready.rows[1].polish.chip, "the mode's own switch is off")
        XCTAssertEqual(ready.rows[0].traits, ["Casual", "Clean up speech"])
        XCTAssertEqual(ready.rows.map(\.title), ["Chat", "Quiet", "Everywhere else"])
        XCTAssertEqual(ready.rows.last?.name, "Default", "the name stays the user's")

        let (off, _) = model(switchOn: false)
        off.apply(listing([mode("c", "Chat"), defaultMode]))
        XCTAssertEqual(off.rows[0].polish, .switchedOff)
        XCTAssertEqual(off.rows[0].polish.chip, "Polish \u{00B7} off")
        XCTAssertTrue(off.rows[0].polish.chipDimmed)
        XCTAssertEqual(off.rows[0].polish.why, "Polish my words is off in AI")
        XCTAssertEqual(off.rows[0].polish.spokenChip, "Polish, off: Polish my words is off in AI")

        let (paused, _) = model()
        paused.apply(listing([mode("c", "Chat"), defaultMode], models: [groq(allowed: false)], setting: "provider:groq"))
        XCTAssertEqual(paused.rows[0].polish.why, "Polish needs your OK again in AI")
        XCTAssertNil(paused.rows[0].polish.rowNote, "the AI setting's OK is asked in AI")

        let (local, _) = model()
        local.apply(listing([mode("c", "Chat", model: "provider:groq", state: "ready"), defaultMode],
                            models: [apple, groq(allowed: true, blocked: true)]))
        XCTAssertEqual(local.rows[0].polish.why, "Local only is on in AI")
        XCTAssertEqual(local.rows[0].polish.rowNote?.text,
                       "Local only is on, so nothing goes to Groq \u{00B7} llama-3.1-8b-instant. Turn Local only off in AI to use it.")
    }

    /// A mode's own model: missing shows no chip and says so; moved and never recorded offer
    /// Confirm…; one no consent covers offers Allow….
    func testAModesOwnModelSaysWhatStopsIt() {
        let (modes, _) = model()
        modes.apply(listing([
            mode("m", "Missing", model: "provider:openai", modelName: "gpt-x", state: "missing"),
            mode("v", "Moved", model: "provider:groq", state: "moved"),
            mode("u", "Unrecorded", model: "provider:groq", modelName: "llama-big", state: "unrecorded"),
            mode("k", "NeedsOK", model: "provider:groq", state: "ready"),
            mode("r", "Ready", model: "engine:apple-foundation-models", state: "ready"),
            mode("x", "Gone engine", model: "engine:someone-else", state: "missing"),
            defaultMode,
        ], models: [apple, groq(allowed: false)]))
        let byName = Dictionary(uniqueKeysWithValues: modes.rows.map { ($0.name, $0.polish) })

        XCTAssertNil(byName["Missing"]?.chip, "its model is gone: no chip")
        XCTAssertEqual(byName["Missing"]?.rowNote?.text,
                       "Its model, OpenAI \u{00B7} gpt-x, isn't available now, so this mode isn't polished. Edit it to pick another.")
        XCTAssertNil(byName["Missing"]?.rowNote?.fix)
        XCTAssertEqual(byName["Gone engine"]?.rowNote?.text,
                       "Its model, a model that was on this Mac, isn't available now, so this mode isn't polished. Edit it to pick another.")

        XCTAssertEqual(byName["Moved"]?.chip, "Polish \u{00B7} off")
        XCTAssertEqual(byName["Moved"]?.why, "Confirm where its model sends")
        XCTAssertEqual(byName["Moved"]?.rowNote?.text,
                       "Its model now sends somewhere else: Groq \u{00B7} llama-3.1-8b-instant. Confirm it to polish with it again.")
        XCTAssertEqual(byName["Moved"]?.rowNote?.fix, .confirm)
        XCTAssertEqual(byName["Unrecorded"]?.rowNote?.text,
                       "Where its model sends was never recorded: Groq \u{00B7} llama-big. Confirm it to polish with it.")
        XCTAssertEqual(byName["Unrecorded"]?.rowNote?.fix, .confirm)

        XCTAssertEqual(byName["NeedsOK"]?.why, "Polish needs your OK to send to Groq")
        XCTAssertEqual(byName["NeedsOK"]?.rowNote?.text, "Polish needs your OK to send this mode's words to Groq.")
        XCTAssertEqual(byName["NeedsOK"]?.rowNote?.fix, .allow)

        XCTAssertEqual(byName["Ready"]?.chip, "Polish")
        XCTAssertNil(byName["Ready"]?.rowNote)
        // Nothing shown is a model's id.
        for row in modes.rows {
            for text in [row.polish.chip, row.polish.why, row.polish.rowNote?.text].compactMap({ $0 }) {
                XCTAssertFalse(text.contains("provider:") || text.contains("engine:"), text)
            }
        }
    }

    /// Verify: the Mac shell says when a mode's own model is missing. The Drop says so at the take
    /// (DictationTests), and Settings reads the modes again, to show which.
    func testATakeWhoseModesModelIsMissingReadsTheModesAgain() {
        let (modes, sent) = model()
        modes.load()
        sent.commands = []
        modes.apply(event(#"{"type":"dictation.warning","kind":"polish_model_missing"}"#))
        XCTAssertEqual(sent.commands, [.modesList(ref: "modes:2")])
    }

    /// A change elsewhere (a consent, a provider, an engine, local-only) reads the modes again, once
    /// Settings has read them: which model each may use has changed.
    func testAChangeElsewhereReadsTheModesAgainOnlyOnceRead() {
        let (modes, sent) = model()
        modes.apply(polishState(on: true))
        XCTAssertEqual(sent.commands, [], "never read: nothing to refresh")
        modes.load()
        modes.apply(polishState(on: true))
        modes.apply(event(#"{"type":"setting.value","key":"llm.local_only","value":"on"}"#))
        modes.apply(event(#"{"type":"engine.unregistered","id":"apple-foundation-models"}"#))
        XCTAssertEqual(sent.commands, [.modesList(ref: "modes:1"), .modesList(ref: "modes:2"),
                                       .modesList(ref: "modes:3"), .modesList(ref: "modes:4")])
    }

    func testDeletingSaysWhereItsAppsGo() throws {
        let (modes, sent) = model()
        modes.apply(listing([mode("c", "Chat", apps: ["com.apple.mail", "com.tinyspeck.slackmacgap"]),
                             mode("n", "Notes", apps: ["com.apple.Notes"]), mode("e", "Empty"), defaultMode]))
        XCTAssertEqual(ModesModel.deleteMessage(modes.rows[0]), "Mail and Slack go back to Everywhere else.")
        XCTAssertEqual(ModesModel.deleteMessage(modes.rows[1]), "Notes goes back to Everywhere else.")
        XCTAssertEqual(ModesModel.deleteMessage(modes.rows[2]), "It has no apps. Voice commands can't switch to it any more.")
        modes.askDelete("d")
        XCTAssertNil(modes.deleting, "the default mode can't be deleted")
        modes.askDelete("c")
        XCTAssertEqual(modes.deleting?.name, "Chat")
        modes.delete(try XCTUnwrap(modes.deleting))
        XCTAssertEqual(sent.commands, [.modesDelete(mode: "c", ref: "modes:1")])
        XCTAssertNil(modes.deleting)
        modes.apply(failed("modes.delete", id: "modes:1", code: "mode_not_found"))
        XCTAssertEqual(modes.problem, "\u{201C}Chat\u{201D} was already deleted.")
    }

    /// The modes could not be read: nothing can be changed, and Start over replaces them.
    func testUnreadableModesOfferStartOver() throws {
        let (modes, sent) = model()
        modes.load()
        modes.apply(failed("modes.list", id: "modes:1", code: nil))
        XCTAssertTrue(modes.failed)
        XCTAssertTrue(modes.unreadable)
        modes.add()
        XCTAssertNil(modes.editor, "no editor over modes that can't be read")
        modes.startOver()
        XCTAssertEqual(sent.saves.count, 0, "not without asking")
        modes.askStartOver()
        XCTAssertTrue(modes.confirmingStartOver)
        modes.startOver()
        let save = try XCTUnwrap(sent.saves.last)
        XCTAssertEqual(save["replace_unreadable"] as? Bool, true)
        XCTAssertEqual((save["mode"] as? [String: Any])?["id"] as? String, "default")
        XCTAssertEqual((save["mode"] as? [String: Any])?.count, 1, "it changes nothing else")
        modes.apply(listing([defaultMode], ref: "modes:2", saved: "d"))
        XCTAssertFalse(modes.failed)
        XCTAssertFalse(modes.unreadable)
    }
}

@MainActor
final class ModesEditorModelTests: XCTestCase {
    private func model(
        _ modes: [String] = [mode("c", "Chat", apps: ["com.tinyspeck.slackmacgap"]), defaultMode],
        models: [String] = [apple], setting: String? = "engine:apple-foundation-models", switchOn: Bool? = true,
        running: [PickedApp] = []
    ) -> (ModesModel, Sent, ConsentModel) {
        let sent = Sent()
        let consent = ConsentModel(feature: .polish, switchSettingID: PolishModel.settingID, send: sent.send)
        let model = ModesModel(send: sent.send, apps: NoApps(), running: FakeRunning(apps: running), consent: consent,
                               polishSwitch: { switchOn })
        model.apply(listing(modes, models: models, setting: setting))
        return (model, sent, consent)
    }

    /// Adding: every field is named, the AI setting's model as null, and the core's answer to
    /// that save closes the editor.
    func testAddingSendsEveryFieldAndTheAnswerClosesTheEditor() throws {
        let (modes, sent, _) = model()
        modes.add()
        let editor = try XCTUnwrap(modes.editor)
        XCTAssertTrue(editor.adding)
        editor.name = "Email"
        editor.style = .relaxed
        editor.polish = true
        editor.prompt = "Keep it short.\r\nNo emoji."
        modes.addApp("com.apple.mail", to: editor)
        modes.addApp("COM.APPLE.MAIL", to: editor)
        XCTAssertEqual(editor.apps, ["com.apple.mail"], "an app once")
        modes.save()
        let save = try XCTUnwrap(sent.saves.last)
        let fields = try XCTUnwrap(save["mode"] as? [String: Any])
        XCTAssertNil(fields["id"], "the core gives a new mode its id")
        XCTAssertEqual(fields["name"] as? String, "Email")
        XCTAssertEqual(fields["style"] as? String, "relaxed")
        XCTAssertEqual(fields["polish"] as? Bool, true)
        XCTAssertEqual(fields["remove_fillers"] as? Bool, true)
        // As typed: the core judges whether it is the default, whatever its line breaks.
        XCTAssertEqual(fields["polish_prompt"] as? String, "Keep it short.\r\nNo emoji.")
        XCTAssertEqual(fields["apps"] as? [String], ["com.apple.mail"])
        XCTAssertTrue(fields["polish_model"] is NSNull, "the AI setting's model")
        XCTAssertTrue(fields["polish_model_name"] is NSNull)
        XCTAssertNil(fields["polish_model_confirm"])
        XCTAssertNil(save["take_apps"])
        XCTAssertTrue(editor.saving)
        modes.save()
        XCTAssertEqual(sent.saves.count, 1, "one save at a time")

        // Another listing (a refresh) is not the save's answer.
        modes.apply(listing([mode("c", "Chat"), defaultMode], ref: "modes:99"))
        XCTAssertNotNil(modes.editor)
        modes.apply(listing([mode("c", "Chat"), mode("m1", "Email"), defaultMode], ref: "modes:1", saved: "m1"))
        XCTAssertNil(modes.editor)
        XCTAssertEqual(modes.rows.map(\.name), ["Chat", "Email", "Default"])
    }

    /// Changing a mode names only what changed: a field it never touched keeps its value, a style
    /// this build doesn't know is kept, and the default mode is never sent apps.
    func testChangingAModeNamesOnlyWhatChanged() throws {
        let (modes, sent, _) = model([mode("c", "Chat", style: "other", prompt: "Mine.", apps: ["com.apple.mail"]), defaultMode])
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        XCTAssertEqual(editor.style, .other)
        XCTAssertFalse(editor.renamed)
        editor.name = "Chat 2"
        XCTAssertTrue(editor.renamed)
        modes.save()
        let fields = try XCTUnwrap(sent.saves.last?["mode"] as? [String: Any])
        XCTAssertEqual(Set(fields.keys), ["id", "name"])
        XCTAssertEqual(fields["id"] as? String, "c")

        modes.closeEditor()
        modes.edit("d")
        let fallback = try XCTUnwrap(modes.editor)
        XCTAssertTrue(fallback.isDefault)
        fallback.name = "Everything"
        modes.addApp("com.apple.mail", to: fallback)
        XCTAssertEqual(fallback.apps, [], "the default mode is every other app's")
        modes.save()
        let renamed = try XCTUnwrap(sent.saves.last?["mode"] as? [String: Any])
        XCTAssertEqual(Set(renamed.keys), ["id", "name"], "the default mode is renamable, and never given apps")
    }

    /// An app another mode has says so, and saving moves it.
    func testPickingAnotherModesAppMovesIt() throws {
        let slack = PickedApp(identity: "com.tinyspeck.slackmacgap", name: "Slack", icon: nil)
        let notes = PickedApp(identity: "com.apple.Notes", name: "Notes", icon: nil)
        let (modes, sent, _) = model(running: [slack, notes])
        modes.add()
        let editor = try XCTUnwrap(modes.editor)
        editor.name = "Work"
        let offers = modes.runningOffers(editor)
        XCTAssertEqual(offers.map(\.app.name), ["Slack", "Notes"])
        XCTAssertEqual(offers.map(\.owner), ["Chat", nil], "Slack — in Chat")
        modes.addApp(slack.identity, to: editor)
        XCTAssertEqual(modes.runningOffers(editor).map(\.app.name), ["Notes"], "not offered twice")
        XCTAssertEqual(modes.movingNotes(editor), ["Moves Slack from Chat."])
        modes.save()
        XCTAssertEqual(sent.saves.last?["take_apps"] as? Bool, true)

        // Removed again: nothing moves.
        modes.apply(failed("modes.save", id: "modes:1", code: "name_taken"))
        modes.removeApp(slack.identity, from: editor)
        XCTAssertEqual(modes.movingNotes(editor), [])
        modes.save()
        XCTAssertNil(sent.saves.last?["take_apps"])
    }

    /// app_taken (another mode took the app meanwhile): said, and the next save moves it.
    func testAnAppTakenMeanwhileIsMovedOnTheNextSave() throws {
        let (modes, sent, _) = model()
        modes.add()
        let editor = try XCTUnwrap(modes.editor)
        editor.name = "Work"
        modes.addApp("com.apple.Notes", to: editor)
        modes.save()
        XCTAssertNil(sent.saves.last?["take_apps"])
        modes.apply(failed("modes.save", id: "modes:1", code: "app_taken"))
        XCTAssertEqual(editor.error, "An app here is in another mode now. Save again to move it here.")
        XCTAssertFalse(editor.saving)
        XCTAssertNotNil(modes.editor, "the editor stays")
        XCTAssertEqual(sent.commands.last, .modesList(ref: "modes:2"), "read again: which mode has it now")
        modes.save()
        XCTAssertEqual(sent.saves.last?["take_apps"] as? Bool, true)
    }

    /// Verify: every refusal the core can give a save, a delete or a confirm is said in words of its
    /// own, never the core's message or its code.
    func testEveryFailureCodeHasItsOwnWords() {
        let editor = ModeEditor(mode: nil, isDefault: false)
        let expected: [FailureCode?: String] = [
            .nameBlank: "Give the mode a name.",
            .nameTaken: "Another mode has a name that sounds the same. Pick another name.",
            .nameIsStyle: "Formal, Casual and Relaxed name the styles in voice commands. Pick another name.",
            .tooLong: "You can have up to 50 modes.",
            .defaultMode: "Everywhere else is used in every app without a mode of its own, so it can't be given apps.",
            .appTaken: "An app here is in another mode now. Save again to move it here.",
            .appInvalid: "One of these apps can't be told apart from others. Remove it, and pick it again.",
            .modeNotFound: "This mode was deleted meanwhile, so it wasn't saved.",
            .modelUnknown: "That model isn't available any more. Pick another.",
            .modelNameInvalid: "A model name can be up to 128 characters, on one line.",
            .destinationChanged: "Where its model sends changed while you looked. Check it, and confirm again.",
            .listUnreadable: "Your modes can't be read, so this wasn't saved. Start over replaces them with the default.",
            .meetingRecording: "Couldn't save the mode. Try again.",
            .deleteWindowOver: "Couldn't save the mode. Try again.",
            nil: "Couldn't save the mode. Try again.",
        ]
        for code in FailureCode.allCases + [nil] {
            let words = ModesModel.saveFailure(code, editor: editor)
            XCTAssertEqual(words, expected[code], "\(String(describing: code))")
            XCTAssertFalse(words.contains("_"), "never the code: \(words)")
        }
        // too_long, by what is too long.
        editor.name = String(repeating: "n", count: 65)
        XCTAssertEqual(ModesModel.saveFailure(.tooLong, editor: editor), "A name can be up to 64 characters.")
        editor.name = "Fine"
        editor.prompt = String(repeating: "p", count: 2001)
        XCTAssertEqual(ModesModel.saveFailure(.tooLong, editor: editor), "Polish instructions can be up to 2,000 characters.")
        editor.prompt = ""
        editor.apps = (0..<65).map { "com.example.app\($0)" }
        XCTAssertEqual(ModesModel.saveFailure(.tooLong, editor: editor), "Shorten to 64 apps or fewer.")
        let imported = ModeEditor(mode: nil, isDefault: false)
        XCTAssertEqual(ModesModel.saveFailure(.tooLong, editor: imported), "You can have up to 50 modes.")

        XCTAssertEqual(ModesModel.deleteFailure(.defaultMode, name: "Default"),
                       "Everywhere else can't be deleted: it is used in every app without a mode of its own.")
        XCTAssertEqual(ModesModel.deleteFailure(.modeNotFound, name: "Chat"), "\u{201C}Chat\u{201D} was already deleted.")
        XCTAssertEqual(ModesModel.deleteFailure(.listUnreadable, name: "Chat"),
                       "Your modes can't be read, so \u{201C}Chat\u{201D} wasn't deleted. Start over replaces them with the default.")
        XCTAssertEqual(ModesModel.deleteFailure(nil, name: "Chat"), "Couldn't delete \u{201C}Chat\u{201D}. Try again.")
        XCTAssertEqual(ModesModel.confirmFailure(.destinationChanged), "Where its model sends changed while you looked. Check it, and confirm again.")
        XCTAssertEqual(ModesModel.confirmFailure(.modeNotFound), "That mode was deleted meanwhile.")
        XCTAssertEqual(ModesModel.confirmFailure(.modelUnknown), "Its model isn't available any more. Edit the mode to pick another.")
        XCTAssertEqual(ModesModel.confirmFailure(.listUnreadable), "Your modes can't be read, so nothing was confirmed. Start over replaces them with the default.")
        XCTAssertEqual(ModesModel.confirmFailure(nil), "Couldn't confirm its model. Try again.")
    }

    /// A refusal reaches the editor that sent it, by its ref, in the code's words; the core's
    /// message (which may name a field) never shows.
    func testARefusalShowsInTheEditorByItsCode() throws {
        let (modes, sent, _) = model()
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        editor.name = "casual"
        modes.save()
        modes.apply(failed("modes.save", id: "modes:other", code: "name_blank"))
        XCTAssertNil(editor.error, "not this save's")
        XCTAssertTrue(editor.saving)
        modes.apply(failed("modes.save", id: "modes:1", code: "name_is_style"))
        XCTAssertEqual(editor.error, ModesModel.saveFailure(.nameIsStyle, editor: editor))
        XCTAssertFalse(editor.error?.contains("zebra") ?? true)
        XCTAssertFalse(editor.saving)
        for code in ["mode_not_found", "model_unknown", "destination_changed"] {
            sent.commands = []
            modes.save()
            let ref = try XCTUnwrap(sent.commands.first?.commandID)
            modes.apply(failed("modes.save", id: ref, code: code))
            XCTAssertTrue(sent.commands.contains { if case .modesList = $0 { true } else { false } }, "\(code): read again")
        }
        // Cancelled while it was on its way: the refusal is said in the section.
        sent.commands = []
        modes.save()
        let late = try XCTUnwrap(sent.commands.first?.commandID)
        modes.closeEditor()
        modes.apply(failed("modes.save", id: late, code: "name_blank"))
        XCTAssertNil(modes.editor)
        XCTAssertEqual(modes.problem, "Your change to \u{201C}Chat\u{201D} wasn't saved. Give the mode a name.")
    }

    /// Picking a model at a destination no polish consent covers asks first; Allow records the OK
    /// (consent.allow, for that destination), and the save follows once the core lists it; Cancel
    /// records and saves nothing.
    func testAModelAtANewDestinationAsksForItsOwnOKBeforeTheSave() throws {
        let (modes, sent, consent) = model(models: [apple, groq(allowed: false)])
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        let options = modes.modelOptions(editor)
        XCTAssertEqual(options.map(\.label), ["As in AI (Apple's on-device model)", "Apple's on-device model", "Groq \u{00B7} llama-3.1-8b-instant"])
        editor.polishModel = "provider:groq"
        XCTAssertEqual(modes.providerModel(editor), "llama-3.1-8b-instant")
        XCTAssertEqual(modes.modelNote(editor).text, "Saving asks for your OK to send this mode's words to Groq.")
        modes.save()
        XCTAssertEqual(editor.consentStep, ConsentModel.Destination(kind: .cloud(endpoint: "https://api.groq.com/openai/v1"), name: "Groq"))
        XCTAssertEqual(modes.consentTitle(editor), "Polish \u{201C}Chat\u{201D} with Groq \u{00B7} llama-3.1-8b-instant?")
        XCTAssertEqual(sent.commands, [], "nothing is sent before Allow")
        modes.cancelConsentStep()
        XCTAssertNil(editor.consentStep)
        XCTAssertEqual(sent.commands, [], "Cancel sends nothing")

        modes.save()
        modes.allowAndSave(try XCTUnwrap(editor.consentStep))
        XCTAssertEqual(sent.commands, [.consentAllow(feature: .polish, to: .cloud, endpoint: "https://api.groq.com/openai/v1", key: nil, ref: "consent.allow:polish:1")])
        XCTAssertTrue(editor.saving)
        // The polish consent model takes the answer too: it is its newest.
        let answer = polishState(on: true, consents: [deviceConsent, groqConsent], ref: "consent.allow:polish:1")
        consent.apply(answer)
        modes.apply(answer)
        XCTAssertEqual(consent.state?.consents.count, 2)
        let fields = try XCTUnwrap(sent.saves.last?["mode"] as? [String: Any])
        XCTAssertEqual(fields["polish_model"] as? String, "provider:groq")
        XCTAssertTrue(fields["polish_model_name"] is NSNull, "the model chosen in AI")

        // A model name at the provider is saved with it; a blank one is the AI setting's.
        editor.polishModelName = " llama-3.3-70b "
        XCTAssertEqual(editor.modelNameToSend, "llama-3.3-70b")
        editor.polishModel = "engine:apple-foundation-models"
        XCTAssertNil(editor.modelNameToSend, "only a provider's")
    }

    /// The OK the core did not record (the model moved while the user read, or the store refused):
    /// nothing is saved, and the editor says so.
    func testAnOKTheCoreDidNotRecordSavesNothing() throws {
        let (modes, sent, _) = model(models: [apple, groq(allowed: false)])
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        editor.polishModel = "provider:groq"
        modes.save()
        modes.allowAndSave(try XCTUnwrap(editor.consentStep))
        modes.apply(polishState(on: true, consents: [deviceConsent], ref: "consent.allow:polish:1"))
        XCTAssertEqual(editor.error, ModesModel.okFailure)
        XCTAssertFalse(editor.saving)
        XCTAssertEqual(sent.saves.count, 0)

        modes.save()
        modes.allowAndSave(try XCTUnwrap(editor.consentStep))
        modes.apply(failed("consent.allow", id: "consent.allow:polish:2", code: nil))
        XCTAssertEqual(editor.error, "Couldn't record your OK, so nothing was saved. Try again.")
        XCTAssertEqual(sent.saves.count, 0)
    }

    /// A mode already on a model whose OK was taken back saves its other changes without asking:
    /// the row offers Allow… instead.
    func testAnUnchangedModelDoesNotAskAgain() throws {
        let (modes, sent, _) = model([mode("c", "Chat", model: "provider:groq", state: "ready"), defaultMode],
                                     models: [apple, groq(allowed: false)])
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        editor.name = "Chats"
        XCTAssertEqual(modes.modelNote(editor).text, "Polish needs your OK to send this mode's words to Groq.")
        XCTAssertTrue(modes.modelNote(editor).isProblem)
        modes.save()
        XCTAssertNil(editor.consentStep)
        XCTAssertEqual(sent.saves.count, 1)
        let keys = (sent.saves[0]["mode"] as? [String: Any]).map { Set($0.keys) }
        XCTAssertEqual(keys, ["id", "name"], "the same pin is not sent back")
    }

    /// Polish on, with no model or with "Polish my words" off: the editor says so under the switch.
    func testTheEditorSaysWhenNothingWouldPolish() throws {
        let (none, _, _) = model(models: [], setting: nil)
        none.edit("c")
        XCTAssertEqual(none.polishNote(try XCTUnwrap(none.editor)), "No language model is available on this Mac, so nothing is polished.")
        XCTAssertEqual(none.modelOptions(try XCTUnwrap(none.editor)).map(\.label), ["As in AI (none now)"])
        let (off, _, _) = model(switchOn: false)
        off.edit("c")
        let editor = try XCTUnwrap(off.editor)
        XCTAssertEqual(off.polishNote(editor), "Polish my words is off in AI, so nothing is polished.")
        editor.polish = false
        XCTAssertNil(off.polishNote(editor))
        XCTAssertEqual(off.modelNote(editor).text, "Uses the model chosen in AI. Your words stay on this Mac.")
    }

    /// A mode's own model that the core no longer holds stays in the picker as it is, named, so
    /// the editor shows what is saved.
    func testAMissingModelStaysInThePicker() throws {
        let (modes, _, _) = model([mode("c", "Chat", model: "provider:openai", state: "missing"), defaultMode])
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        XCTAssertEqual(modes.modelOptions(editor).last?.label, "OpenAI (not available)")
        XCTAssertEqual(modes.modelOptions(editor).last?.id, "provider:openai")
        XCTAssertEqual(modes.modelNote(editor).text, "This model isn't available now, so this mode isn't polished. Pick another.")
        XCTAssertTrue(modes.modelNote(editor).isProblem)
        XCTAssertFalse(modes.canConfirmInEditor(editor), "nothing to confirm: it isn't there")
    }

    /// A row's Confirm…: where its model sends now, agreed to, saved as the confirm alone; a
    /// destination no consent covers asks for its OK in the same step.
    func testConfirmingAMovedModelFromTheRow() throws {
        let (modes, sent, _) = model([mode("c", "Chat", model: "provider:groq", state: "moved"), defaultMode],
                                     models: [apple, groq(allowed: true)])
        modes.askConfirm("c")
        let c = try XCTUnwrap(modes.confirming)
        XCTAssertTrue(c.pin)
        XCTAssertFalse(c.asksOK)
        XCTAssertEqual(ModesModel.confirmTitle(c), "Polish \u{201C}Chat\u{201D} with Groq \u{00B7} llama-3.1-8b-instant?")
        XCTAssertEqual(ModesModel.confirmButton(c), "Send to Groq")
        XCTAssertEqual(modes.confirmMessage(c), ConsentModel.message(.polish, c.choice.destination))
        modes.confirmAllow(try XCTUnwrap(modes.confirming))
        XCTAssertNil(modes.confirming)
        let save = try XCTUnwrap(sent.saves.last)
        let fields = try XCTUnwrap(save["mode"] as? [String: Any])
        XCTAssertEqual(Set(fields.keys), ["id", "polish_model_confirm", "polish_model_confirm_to"])
        XCTAssertEqual(fields["polish_model_confirm"] as? Bool, true)
        XCTAssertEqual(fields["polish_model_confirm_to"] as? [String: String], ["to": "cloud", "endpoint": "https://api.groq.com/openai/v1"])
        modes.apply(failed("modes.save", id: "modes:1", code: "destination_changed"))
        XCTAssertEqual(modes.problem, ModesModel.confirmFailure(.destinationChanged))
        XCTAssertEqual(sent.commands.last, .modesList(ref: "modes:2"), "read again, to ask again")

        // Not covered: the OK first, then the confirm.
        let (asking, asked, _) = model([mode("c", "Chat", model: "provider:groq", state: "unrecorded"), defaultMode],
                                       models: [apple, groq(allowed: false)], switchOn: false)
        asking.askConfirm("c")
        let both = try XCTUnwrap(asking.confirming)
        XCTAssertTrue(both.asksOK)
        XCTAssertTrue(asking.confirmMessage(both).hasSuffix(" This also turns on Polish my words."))
        asking.confirmAllow(try XCTUnwrap(asking.confirming))
        XCTAssertEqual(asked.commands, [.consentAllow(feature: .polish, to: .cloud, endpoint: "https://api.groq.com/openai/v1", key: nil, ref: "consent.allow:polish:1")])
        asking.apply(polishState(on: true, consents: [groqConsent], ref: "consent.allow:polish:1"))
        XCTAssertEqual((asked.saves.last?["mode"] as? [String: Any])?["polish_model_confirm"] as? Bool, true)
    }

    /// A row's Allow…: the OK for a mode's own model's destination, and nothing else.
    func testAllowingFromTheRowRecordsOnlyTheOK() throws {
        let (modes, sent, _) = model([mode("c", "Chat", model: "provider:groq", state: "ready"), defaultMode],
                                     models: [apple, groq(allowed: false)])
        modes.askConfirm("c")
        XCTAssertEqual(modes.confirming?.pin, false)
        modes.confirmAllow(try XCTUnwrap(modes.confirming))
        modes.apply(polishState(on: true, consents: [groqConsent], ref: "consent.allow:polish:1"))
        XCTAssertEqual(sent.saves.count, 0)
        XCTAssertNil(modes.problem)
        // Refused: said in the section.
        modes.askConfirm("c")
        modes.confirmAllow(try XCTUnwrap(modes.confirming))
        modes.apply(polishState(on: true, consents: [], ref: "consent.allow:polish:2"))
        XCTAssertEqual(modes.problem, ModesModel.okFailureRow)
    }

    /// The editor's Confirm: the save then confirms where the mode's model sends now.
    func testConfirmingInTheEditor() throws {
        let (modes, sent, _) = model([mode("c", "Chat", model: "engine:apple-foundation-models", state: "unrecorded"), defaultMode])
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        XCTAssertTrue(modes.canConfirmInEditor(editor))
        XCTAssertEqual(modes.modelNote(editor).isProblem, true)
        modes.confirmInEditor(editor)
        XCTAssertTrue(editor.confirmed)
        XCTAssertFalse(modes.canConfirmInEditor(editor))
        modes.save()
        let fields = try XCTUnwrap(sent.saves.last?["mode"] as? [String: Any])
        XCTAssertEqual(fields["polish_model_confirm_to"] as? [String: String], ["to": "on_device"])
        XCTAssertNil(fields["polish_model"], "the same pin, confirmed")
    }

    /// The editor's Confirm sends the destination the user was shown, never one looked up again
    /// at the save; refused because it sends elsewhere now, Confirm asks again.
    func testAnEditorConfirmSendsWhatWasShownAndAsksAgainWhenItMoved() throws {
        let (modes, sent, _) = model([mode("c", "Chat", model: "provider:groq", state: "moved"), defaultMode],
                                     models: [apple, groq(allowed: true)])
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        modes.confirmInEditor(editor)
        XCTAssertEqual(editor.confirmedTo?.destination.kind, .cloud(endpoint: "https://api.groq.com/openai/v1"))
        // The provider is re-pointed while the editor is open: a listing names another endpoint.
        let moved = #"{"id":"provider:groq","name":"Groq","model":"llama-3.1-8b-instant","to":"cloud","endpoint":"https://elsewhere.example.com/v1","allowed":true,"blocked_local_only":false}"#
        modes.apply(listing([mode("c", "Chat", model: "provider:groq", state: "moved"), defaultMode], models: [apple, moved]))
        modes.save()
        let fields = try XCTUnwrap(sent.saves.last?["mode"] as? [String: Any])
        XCTAssertEqual((fields["polish_model_confirm_to"] as? [String: String])?["endpoint"], "https://api.groq.com/openai/v1",
                       "what the user was shown, for the core to refuse")
        modes.apply(failed("modes.save", id: "modes:1", code: "destination_changed"))
        XCTAssertFalse(editor.confirmed, "not confirmed any more")
        XCTAssertTrue(modes.canConfirmInEditor(editor), "Confirm asks again")
        XCTAssertEqual(editor.error, ModesModel.saveFailure(.destinationChanged, editor: editor))
        modes.save()
        XCTAssertNil((sent.saves.last?["mode"] as? [String: Any])?["polish_model_confirm"], "no confirm without a new one")
    }

    /// A confirmation the listing has since overtaken (the model moved again) no longer counts:
    /// Confirm asks again.
    func testAConfirmationAListingOvertookAsksAgain() throws {
        let (modes, _, _) = model([mode("c", "Chat", model: "provider:groq", state: "moved"), defaultMode],
                                  models: [apple, groq(allowed: true)])
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        modes.confirmInEditor(editor)
        XCTAssertTrue(modes.confirmed(editor))
        let moved = #"{"id":"provider:groq","name":"Groq","model":"llama-3.1-8b-instant","to":"cloud","endpoint":"https://elsewhere.example.com/v1","allowed":true,"blocked_local_only":false}"#
        modes.apply(listing([mode("c", "Chat", model: "provider:groq", state: "moved"), defaultMode], models: [apple, moved]))
        XCTAssertFalse(modes.confirmed(editor))
        XCTAssertTrue(modes.canConfirmInEditor(editor))
    }

    /// Two editors' saves in flight: each refusal reaches its own sender.
    func testOverlappingSavesKeepTheirOwnRefusals() throws {
        let (modes, _, _) = model()
        modes.edit("c")
        let first = try XCTUnwrap(modes.editor)
        first.name = "Casual"
        modes.save()
        modes.closeEditor()
        modes.add()
        let second = try XCTUnwrap(modes.editor)
        second.name = ""
        modes.save()
        modes.apply(failed("modes.save", id: "modes:1", code: "name_is_style"))
        XCTAssertEqual(modes.problem, "Your change to \u{201C}Chat\u{201D} wasn't saved. \(ModesModel.saveFailure(.nameIsStyle, editor: first))")
        modes.apply(failed("modes.save", id: "modes:2", code: "name_blank"))
        XCTAssertEqual(second.error, "Give the mode a name.")
    }

    /// The core stopped with something in flight: nothing waits for an answer that won't come.
    func testACoreThatStoppedLeavesNothingWaiting() throws {
        let (modes, _, _) = model([mode("c", "Chat"), mode("n", "Notes"), defaultMode])
        modes.askDelete("c")
        modes.delete(try XCTUnwrap(modes.deleting))
        modes.edit("n")
        let editor = try XCTUnwrap(modes.editor)
        modes.save()
        XCTAssertTrue(modes.busy)
        XCTAssertTrue(editor.saving)
        modes.apply(event(#"{"type":"core.stopped"}"#))
        XCTAssertFalse(modes.busy)
        XCTAssertFalse(editor.saving)
    }

    /// Cancelled while its save was on its way: a refusal is said in the section, not dropped.
    func testASaveRefusedAfterItsEditorClosedIsSaid() throws {
        let (modes, _, _) = model()
        modes.edit("c")
        let editor = try XCTUnwrap(modes.editor)
        editor.name = "Casual"
        modes.save()
        modes.closeEditor()
        modes.apply(failed("modes.save", id: "modes:1", code: "name_is_style"))
        XCTAssertNil(modes.editor)
        XCTAssertEqual(modes.problem, "Your change to \u{201C}Chat\u{201D} wasn't saved. \(ModesModel.saveFailure(.nameIsStyle, editor: editor))")
        // A save that lands after its editor closed changes nothing else.
        modes.add()
        let other = try XCTUnwrap(modes.editor)
        other.name = "Notes"
        modes.save()
        modes.closeEditor()
        modes.add()
        let third = try XCTUnwrap(modes.editor)
        modes.apply(listing([mode("c", "Chat"), mode("n", "Notes"), defaultMode], ref: "modes:2", saved: "n"))
        XCTAssertTrue(modes.editor === third, "another editor opened since stays open")
    }

    /// One row operation at a time: a second tap while the first waits never takes its place.
    func testRowOperationsWaitForTheirAnswer() throws {
        let (modes, sent, _) = model([mode("c", "Chat"), mode("n", "Notes"), defaultMode])
        modes.askDelete("c")
        modes.delete(try XCTUnwrap(modes.deleting))
        XCTAssertTrue(modes.busy)
        modes.askDelete("n")
        XCTAssertNil(modes.deleting, "waits for the first")
        modes.apply(failed("modes.delete", id: "modes:1", code: nil))
        XCTAssertFalse(modes.busy)
        XCTAssertEqual(modes.problem, "Couldn't delete \u{201C}Chat\u{201D}. Try again.")
        modes.askDelete("n")
        modes.delete(try XCTUnwrap(modes.deleting))
        modes.apply(listing([mode("c", "Chat"), defaultMode], ref: "modes:2"))
        XCTAssertFalse(modes.busy)
        XCTAssertNil(modes.problem, "a stale failure goes once something succeeds")
        XCTAssertEqual(sent.commands.filter { if case .modesDelete = $0 { true } else { false } }.count, 2)
    }

    /// "Save again to move it" is for the next save only.
    func testTakingAppsIsForTheNextSaveOnly() throws {
        let (modes, sent, _) = model()
        modes.add()
        let editor = try XCTUnwrap(modes.editor)
        editor.name = "Work"
        modes.addApp("com.apple.Notes", to: editor)
        modes.save()
        modes.apply(failed("modes.save", id: "modes:1", code: "app_taken"))
        modes.save()
        XCTAssertEqual(sent.saves.last?["take_apps"] as? Bool, true)
        modes.apply(failed("modes.save", id: "modes:3", code: "name_taken"))
        modes.save()
        XCTAssertNil(sent.saves.last?["take_apps"], "said once, used once")
    }

    /// Each app is named once (Launch Services reads files), not at every draw.
    func testAnAppIsLookedUpOnce() throws {
        final class Counting: AppDirectory, @unchecked Sendable {
            var asked = 0
            func app(bundleID: String) -> (name: String, icon: NSImage)? {
                asked += 1
                return bundleID == "com.apple.mail" ? ("Mail", NSImage()) : nil
            }
        }
        let apps = Counting()
        let modes = ModesModel(send: { _ in }, apps: apps)
        modes.apply(listing([mode("c", "Chat", apps: ["com.apple.mail", "com.example.gone"]), defaultMode]))
        XCTAssertEqual(apps.asked, 2)
        for _ in 0..<5 {
            _ = modes.label("com.apple.mail")
            _ = modes.label("com.example.gone")
        }
        XCTAssertEqual(apps.asked, 2, "not at each draw")
        // A new listing asks again about an app not on this Mac (it may have been installed since).
        modes.apply(listing([mode("c", "Chat", apps: ["com.apple.mail", "com.example.gone"]), defaultMode]))
        XCTAssertEqual(apps.asked, 3)
    }

    func testAnOKIsMatchedWhateverTheEndpointsTrailingSlash() {
        let state = ConsentModel.Snapshot(on: true, allowed: true, destination: nil, error: nil,
                                          consents: [ConsentModel.Granted(kind: .cloud(endpoint: "https://api.groq.com/openai/v1"), name: "Groq")])
        XCTAssertTrue(state.covers(ConsentModel.Destination(kind: .cloud(endpoint: "https://api.groq.com/openai/v1/"), name: "Groq")))
        XCTAssertFalse(state.covers(ConsentModel.Destination(kind: .cloud(endpoint: "https://api.groq.com/openai"), name: "Groq")))
        XCTAssertFalse(state.covers(ConsentModel.Destination(kind: .onDevice, name: "x")))
    }

    func testTooLongFallsBackToPlainWords() {
        let editor = ModeEditor(mode: nil, isDefault: false)
        editor.name = String(repeating: "\u{1F600}", count: 40)
        XCTAssertEqual(ModesModel.saveFailure(.tooLong, editor: editor), "You can have up to 50 modes.")
        let existing = ModeEditor(mode: try? JSONDecoder().decode(ModeInfo.self, from: Data(mode("c", "Chat").utf8)), isDefault: false)
        XCTAssertEqual(ModesModel.saveFailure(.tooLong, editor: existing), "Something here is too long. Shorten it.")
        // Counted as the core counts: a flag is two scalars.
        existing.prompt = String(repeating: "\u{1F1E9}\u{1F1F0}", count: 1001)
        XCTAssertEqual(ModesModel.saveFailure(.tooLong, editor: existing), "Polish instructions can be up to 2,000 characters.")
        XCTAssertTrue(existing.promptTooLong)
    }

    func testTheEditorsCountAndPlaceholder() throws {
        let (modes, _, _) = model()
        modes.add()
        let editor = try XCTUnwrap(modes.editor)
        XCTAssertEqual(modes.defaultPrompt, "Tidy it.")
        editor.prompt = String(repeating: "a", count: 120)
        XCTAssertEqual(editor.promptCount, "\(120.formatted()) / \(2000.formatted())")
        XCTAssertFalse(editor.promptTooLong)
        editor.prompt = String(repeating: "a", count: 2001)
        XCTAssertTrue(editor.promptTooLong)
    }

    func testAChosenAppIsAddedByItsBundleID() throws {
        let (modes, _, _) = model()
        modes.add()
        let editor = try XCTUnwrap(modes.editor)
        modes.addChosen(URL(fileURLWithPath: "/System/Applications/TextEdit.app"), to: editor)
        XCTAssertEqual(editor.apps, ["com.apple.TextEdit"])
        modes.addChosen(URL(fileURLWithPath: "/System/Library/CoreServices/no-such.app"), to: editor)
        XCTAssertEqual(editor.error, "That app has no bundle identifier, so Inkwell can't tell when it's in front.")
        XCTAssertEqual(editor.apps, ["com.apple.TextEdit"])
    }

    /// Running now: regular apps only, Inkwell left out, by name.
    func testRunningNowLeavesInkwellOut() {
        let running = WorkspaceRunningApps().running()
        XCTAssertFalse(running.contains { $0.identity.caseInsensitiveCompare(WorkspaceRunningApps.inkwell) == .orderedSame })
        XCTAssertEqual(running.map(\.name), running.map(\.name).sorted { $0.localizedStandardCompare($1) == .orderedAscending })
        XCTAssertEqual(Set(running.map { $0.identity.lowercased() }).count, running.count)
    }
}

/// The editor's commands against the real core: the fields it sends are the ones the core reads,
/// and each refusal comes back with the code the editor's words are chosen by. A fresh library in
/// a temporary directory, with no language model.
@MainActor
final class ModesCoreRoundTripTests: XCTestCase {
    func testTheEditorAgainstTheRealCore() throws {
        let data = FileManager.default.temporaryDirectory.appendingPathComponent("inkwell-modes-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: data) }
        let events = Mutex<[InkEvent]>([])
        let session = try InkSession.start(InkConfig(dataDir: data.path, logLevel: "warn")) { event in
            events.withLock { $0.append(event) }
        }
        defer { session.shutdown() }
        let modes = ModesModel(send: { command in try? session.command(command.json) }, apps: NoApps(),
                               running: FakeRunning(), polishSwitch: { true })
        func pump(_ what: String, until done: () -> Bool) {
            let deadline = Date().addingTimeInterval(10)
            while Date() < deadline {
                for event in events.withLock({ let all = $0; $0 = []; return all }) {
                    modes.apply(event)
                }
                if done() { return }
                Thread.sleep(forTimeInterval: 0.01)
            }
            XCTFail("never: \(what)")
        }
        func saveRefused(_ what: String) -> String? {
            modes.save()
            pump(what) { modes.editor?.saving == false }
            return modes.editor?.error
        }

        modes.load()
        pump("listed") { !modes.rows.isEmpty }
        XCTAssertEqual(modes.rows.map(\.name), ["Default"])
        XCTAssertFalse(modes.defaultPrompt.isEmpty)
        XCTAssertNil(modes.rows[0].polish.chip, "no language model here: no Polish chip")

        // Added, with every field the editor sends.
        modes.add()
        var editor = try XCTUnwrap(modes.editor)
        editor.name = "Chat"
        editor.style = .casual
        editor.polish = true
        editor.prompt = "Short.\r\nNo emoji."
        modes.addApp("com.example.chat", to: editor)
        modes.save()
        pump("saved") { modes.editor == nil }
        XCTAssertEqual(modes.rows.map(\.name), ["Chat", "Default"])
        XCTAssertEqual(modes.rows[0].apps.map(\.id), ["com.example.chat"])
        XCTAssertEqual(modes.rows[1].title, "Everywhere else")
        let chat = modes.rows[0].id

        // Each refusal, by its code.
        modes.edit(chat)
        editor = try XCTUnwrap(modes.editor)
        editor.name = "  "
        XCTAssertEqual(saveRefused("name_blank"), ModesModel.saveFailure(.nameBlank, editor: editor))
        editor.name = "Casual"
        XCTAssertEqual(saveRefused("name_is_style"), ModesModel.saveFailure(.nameIsStyle, editor: editor))
        editor.name = "default "
        XCTAssertEqual(saveRefused("name_taken"), ModesModel.saveFailure(.nameTaken, editor: editor))
        editor.name = String(repeating: "n", count: 65)
        XCTAssertEqual(saveRefused("too_long name"), "A name can be up to 64 characters.")
        editor.name = "Chat"
        editor.prompt = String(repeating: "p", count: 2001)
        XCTAssertEqual(saveRefused("too_long prompt"), "Polish instructions can be up to 2,000 characters.")
        editor.prompt = "Short."
        editor.apps = ["x"]
        XCTAssertEqual(saveRefused("app_invalid"), ModesModel.saveFailure(.appInvalid, editor: editor))
        editor.apps = ["com.example.chat"]
        editor.polishModel = "provider:groq"
        XCTAssertEqual(saveRefused("model_unknown"), ModesModel.saveFailure(.modelUnknown, editor: editor))
        editor.polishModel = nil
        modes.save()
        pump("saved after the refusals") { modes.editor == nil }

        // The default mode is renamable, and keeps no apps.
        modes.edit("default")
        try XCTUnwrap(modes.editor).name = "Everything"
        modes.save()
        pump("default renamed") { modes.editor == nil }
        XCTAssertEqual(modes.rows.last?.name, "Everything")
        XCTAssertEqual(modes.rows.last?.title, "Everywhere else")

        // An app from another mode moves.
        modes.add()
        editor = try XCTUnwrap(modes.editor)
        editor.name = "Work"
        modes.addApp("com.example.chat", to: editor)
        XCTAssertEqual(modes.movingNotes(editor), ["Moves An app not on this Mac from Chat."])
        modes.save()
        pump("moved") { modes.editor == nil }
        XCTAssertEqual(modes.rows.first { $0.name == "Chat" }?.apps.map(\.id), [])
        XCTAssertEqual(modes.rows.first { $0.name == "Work" }?.apps.map(\.id), ["com.example.chat"])

        // A mode deleted under an open editor.
        let work = try XCTUnwrap(modes.rows.first { $0.name == "Work" }).id
        modes.edit(work)
        editor = try XCTUnwrap(modes.editor)
        modes.askDelete(work)
        modes.delete(try XCTUnwrap(modes.deleting))
        pump("deleted") { !modes.rows.contains { $0.id == work } }
        editor.name = "Work again"
        XCTAssertEqual(saveRefused("mode_not_found"), ModesModel.saveFailure(.modeNotFound, editor: editor))
        modes.closeEditor()

        // A confirm the core reads, refused here only because nothing is held.
        let chatRow = try XCTUnwrap(modes.rows.first { $0.name == "Chat" })
        XCTAssertNil(chatRow.polish.rowNote)

        // Polish's consents: a revoke of one not there is no failure.
        let consent = ConsentModel(feature: .polish, switchSettingID: PolishModel.settingID, send: { try? session.command($0.json) })
        consent.revoke(ConsentModel.Granted(kind: .onDevice, name: nil))
        let deadline = Date().addingTimeInterval(10)
        var answered = false
        while Date() < deadline, !answered {
            for event in events.withLock({ let all = $0; $0 = []; return all }) {
                consent.apply(event)
                if case .consentState(let state) = event, state.ref == "consent.revoke:polish:1" { answered = true }
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        XCTAssertTrue(answered)
        XCTAssertNil(consent.failure)
        XCTAssertEqual(consent.state?.consents, [])
    }
}
