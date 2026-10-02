// No speech model yet: with nothing installed that writes speech down, Today, the Drop and Live
// say so (rather than "Hold fn to dictate", "The microphone is silent" and "Waiting for someone to
// speak"), Today offers the download (the Drop a button to it), and the normal lines come back
// the moment a model is in. The state is the router's answers (engine.routed) and the catalogue's list.
import Foundation
import InkBridge
import XCTest

@testable import Inkwell

private func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    guard let decoded = try? InkEvent.decode(Data(json.utf8)) else {
        XCTFail("not an event: \(json)", file: file, line: line)
        return .unknown(type: "")
    }
    return decoded
}

@MainActor
private final class Sent {
    var commands: [CoreCommand] = []
    lazy var send: SendCommand = { [unowned self] in self.commands.append($0) }
}

@MainActor
final class SpeechModelsTests: XCTestCase {
    private let silero = "silero-vad-v6-16k"
    private let parakeet = "parakeet-tdt-0.6b-v3-coreml"
    private let qwen = "qwen3-asr-1.7b-q8"

    /// The Mac's catalogue: voice detection, Parakeet (a shell engine: no rates of its own) and
    /// Qwen3-ASR, each installed or not.
    private func listed(silero: Bool = false, parakeet: Bool = false, qwen: Bool = false) -> InkEvent {
        event(#"{"type":"models.listed","models":[{"id":"silero-vad-v6-16k","licence":"MIT","size_bytes":1289603,"installed":\#(silero),"jobs":[{"job":"voice_activity","wer":0}]},{"id":"parakeet-tdt-0.6b-v3-coreml","licence":"CC-BY-4.0","size_bytes":483105645,"installed":\#(parakeet),"jobs":[]},{"id":"qwen3-asr-1.7b-q8","licence":"Apache-2.0","size_bytes":2520744288,"installed":\#(qwen),"jobs":[{"job":"dictation_final","wer":4.59},{"job":"meeting_final","wer":16.08}]}]}"#)
    }

    private func routed(_ job: String, _ id: String? = nil, source: String = "shell") -> InkEvent {
        guard let id else { return event(#"{"type":"engine.routed","job":"\#(job)"}"#) }
        return event(#"{"type":"engine.routed","job":"\#(job)","id":"\#(id)","source":"\#(source)"}"#)
    }

    /// Nothing installed: the list and the router agree that nothing writes speech down.
    private func nothingInstalled(_ catalogue: CatalogueModel) {
        catalogue.apply(listed())
        catalogue.apply(routed("dictation_final"))
        catalogue.apply(routed("meeting_final"))
        catalogue.apply(routed("live_partials"))
    }

    // MARK: The state

    func testTheStateIsNoModelOnlyWhenTheListAndTheRouterBothSaySo() {
        let catalogue = CatalogueModel(send: { _ in })
        XCTAssertEqual(catalogue.speech, .unknown, "nothing answered yet: no claim")
        catalogue.apply(routed("dictation_final"))
        catalogue.apply(routed("meeting_final"))
        XCTAssertEqual(catalogue.speech.dictation, .unknown, "the list is not known yet")
        catalogue.apply(listed())
        XCTAssertEqual(catalogue.speech.dictation, .missing)
        XCTAssertEqual(catalogue.speech.meetings, .missing)

        // Parakeet is on disk, but the shell has not registered it yet (it loads at launch, and
        // after its download): no claim until the router answers again.
        catalogue.apply(listed(parakeet: true))
        XCTAssertEqual(catalogue.speech.dictation, .unknown)
        XCTAssertEqual(catalogue.speech.meetings, .unknown)

        // A route question that failed is not "nothing installed".
        catalogue.apply(listed())
        catalogue.apply(event(#"{"type":"command.failed","command":"engine.route","id":"\#(CoreCommand.engineRoute(.dictationFinal).commandID ?? "")","message":"x"}"#))
        XCTAssertEqual(catalogue.speech.dictation, .unknown)
        XCTAssertEqual(catalogue.speech.meetings, .missing)
    }

    func testDictationOnlyAndInstalled() {
        let catalogue = CatalogueModel(send: { _ in })
        catalogue.apply(listed())
        catalogue.apply(routed("dictation_final", "example-dictation", source: "registry"))
        catalogue.apply(routed("meeting_final"))
        XCTAssertEqual(catalogue.speech.dictation, .served)
        XCTAssertEqual(catalogue.speech.meetings, .missing)
        catalogue.apply(routed("meeting_final", "example-meetings", source: "registry"))
        XCTAssertEqual(catalogue.speech.dictation, .served)
        XCTAssertEqual(catalogue.speech.meetings, .served)
    }

    /// The download: the recommended set (voice detection and Parakeet, about 484 MB), the models
    /// of it not on this Mac, smallest first, and nothing more. When it is in and the shell has
    /// registered Parakeet, the state is served again, with no restart.
    func testTheDownloadFetchesTheRecommendedSetAndTheStateComesBackWhenItIsIn() {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        nothingInstalled(catalogue)
        XCTAssertEqual(catalogue.recommendedMB, 484, "about 484 MB")
        XCTAssertEqual(catalogue.speechDownload, .notStarted)
        sent.commands = []

        catalogue.downloadRecommended()
        XCTAssertEqual(sent.commands, [.modelInstall(silero, ref: "model.update:1")], "one at a time, smallest first")
        XCTAssertTrue(catalogue.speech.downloading)
        catalogue.apply(event(#"{"type":"model.update_progress","id":"\#(silero)","next":"\#(silero)","done_bytes":1289603,"total_bytes":1289603}"#))
        catalogue.apply(event(#"{"type":"model.update_finished","id":"\#(silero)","next":"\#(silero)","ok":true,"no_model_warm":false}"#))
        XCTAssertTrue(sent.commands.contains(.modelInstall(parakeet, ref: "model.update:2")))
        catalogue.apply(event(#"{"type":"model.update_progress","id":"\#(parakeet)","next":"\#(parakeet)","done_bytes":241552822,"total_bytes":483105645}"#))
        XCTAssertEqual(catalogue.speechDownload, .downloading(percent: 50))
        XCTAssertFalse(sent.commands.contains { if case .modelInstall(let id, _) = $0 { id == self.qwen } else { false } },
                       "Qwen3-ASR is not in the set")

        catalogue.apply(event(#"{"type":"model.update_finished","id":"\#(parakeet)","next":"\#(parakeet)","ok":true,"no_model_warm":false}"#))
        catalogue.apply(listed(silero: true, parakeet: true))
        XCTAssertEqual(catalogue.speech.dictation, .unknown, "on disk, the shell is loading it")
        catalogue.apply(event(#"{"type":"engine.registered","id":"fluidaudio-parakeet-tdt-0.6b-v3-offline","kind":"offline","jobs":[{"job":"dictation_final","wer":6.7},{"job":"meeting_final","wer":23.4}]}"#))
        catalogue.apply(routed("dictation_final", "fluidaudio-parakeet-tdt-0.6b-v3-offline"))
        catalogue.apply(routed("meeting_final", "fluidaudio-parakeet-tdt-0.6b-v3-offline"))
        XCTAssertEqual(catalogue.speech, SpeechModels(dictation: .served, meetings: .served))
        XCTAssertNil(SpeechModels.todayLine(catalogue.speech), "the normal lines are back")
    }

    func testAFailedDownloadSaysSoAndCanBeTriedAgain() {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        nothingInstalled(catalogue)
        catalogue.downloadRecommended()
        catalogue.apply(event(#"{"type":"model.update_finished","id":"\#(silero)","next":"\#(silero)","ok":false,"no_model_warm":false,"message":"the connection was reset"}"#))
        catalogue.apply(event(#"{"type":"model.update_finished","id":"\#(parakeet)","next":"\#(parakeet)","ok":true,"no_model_warm":false}"#))
        XCTAssertEqual(catalogue.speechDownload, .failed("the connection was reset"))
        catalogue.downloadRecommended()
        XCTAssertEqual(catalogue.speechDownload, .downloading(percent: nil))
    }

    /// A model whose download just finished counts as on this Mac before the list says so (it is
    /// asked for again at the finish, and answers later): a press in between fetches only the rest,
    /// and a model whose download failed is tried again.
    func testAModelThatJustFinishedIsNotFetchedAgainAndAFailedOneIsTriedAgain() {
        let sent = Sent()
        let catalogue = CatalogueModel(send: sent.send)
        nothingInstalled(catalogue)
        catalogue.downloadRecommended()
        catalogue.apply(event(#"{"type":"model.update_finished","id":"\#(silero)","next":"\#(silero)","ok":false,"no_model_warm":false,"message":"the connection was reset"}"#))
        catalogue.apply(event(#"{"type":"model.update_finished","id":"\#(parakeet)","next":"\#(parakeet)","ok":true,"no_model_warm":false}"#))
        sent.commands = []

        catalogue.downloadRecommended()
        XCTAssertEqual(sent.commands, [.modelInstall(silero, ref: "model.update:3")], "the failed one again, not Parakeet")
    }

    // MARK: The words

    func testTodaysLineAndTheHoldHint() {
        XCTAssertNil(SpeechModels.todayLine(.unknown))
        XCTAssertNil(SpeechModels.todayLine(SpeechModels(dictation: .served, meetings: .served)))
        XCTAssertEqual(SpeechModels.todayLine(SpeechModels(dictation: .missing, meetings: .missing)),
                       "No speech model yet, so nothing you say can be written down.")
        XCTAssertEqual(SpeechModels.todayLine(SpeechModels(dictation: .served, meetings: .missing)),
                       "No speech model for meetings yet, so they are recorded but not transcribed.")
        XCTAssertEqual(SpeechModels.todayLine(SpeechModels(dictation: .missing, meetings: .served)),
                       "No speech model for dictation yet.")
        XCTAssertFalse(SpeechModels(dictation: .missing, meetings: .missing).offersDictation, "no \u{201C}Hold fn to dictate\u{201D}")
        XCTAssertTrue(SpeechModels(dictation: .served, meetings: .missing).offersDictation)
        XCTAssertTrue(SpeechModels.unknown.offersDictation, "not known: the usual line")
    }

    /// After a hold with no model, the Drop says so, with a button to the download, never that the
    /// microphone is silent; with a model, the microphone's note is unchanged.
    func testTheDropAfterAHoldSaysThereIsNoSpeechModel() throws {
        let none = SpeechModels(dictation: .missing, meetings: .missing)
        let takes = [
            #"{"type":"dictation.discarded","reason":"silence"}"#,
            #"{"type":"dictation.discarded","reason":"no_speech"}"#,
            #"{"type":"dictation.discarded","reason":"nothing_heard"}"#,
            #"{"type":"dictation.failed","stage":"transcription","message":"model not installed"}"#,
        ]
        for json in takes {
            let note = try XCTUnwrap(DictationModel.note(for: event(json), hasLanguageModel: false, speech: none), json)
            XCTAssertEqual(note.title, "No speech model yet", json)
            XCTAssertEqual(note.detail, "Nothing can be typed until one is installed", json)
            XCTAssertEqual(note.actions, [.showSpeechModels], json)
        }
        var downloading = none
        downloading.downloading = true
        let waiting = try XCTUnwrap(DictationModel.note(for: event(takes[0]), hasLanguageModel: false, speech: downloading))
        XCTAssertEqual(waiting.title, "The speech model is downloading")
        XCTAssertEqual(waiting.actions, [], "already asked for")

        let installed = SpeechModels(dictation: .served, meetings: .served)
        XCTAssertEqual(DictationModel.note(for: event(takes[0]), hasLanguageModel: false, speech: installed)?.title,
                       "The microphone is silent")
        XCTAssertEqual(DictationModel.note(for: event(takes[0]), hasLanguageModel: false, speech: .unknown)?.title,
                       "The microphone is silent", "not known: no claim")
        XCTAssertEqual(DictationModel.note(for: event(#"{"type":"dictation.discarded","reason":"too_short"}"#), hasLanguageModel: false, speech: none)?.title,
                       "Too short", "still true without a model")
    }

    /// The models the screens read: the Drop's note comes through DictationModel with the
    /// catalogue's state. Its button downloads nothing: a download starts only where its size and
    /// hosts are shown, so it brings the main window to Today, whose line has both. The note is
    /// left as it is, up for the time a note with a button gets.
    func testTheDropsButtonOpensTodayAndDownloadsNothing() throws {
        let sent = Sent()
        let screens = ScreenModels(send: sent.send, calendar: NoCalendar(), apps: WorkspaceApps())
        nothingInstalled(screens.catalogue)
        screens.apply([event(#"{"type":"dictation.discarded","reason":"silence"}"#)])
        let note = try XCTUnwrap(screens.dictation.note)
        XCTAssertEqual(note.text.title, "No speech model yet")
        XCTAssertEqual(note.text.actions.map(\.title), ["Download speech models\u{2026}"], "the ellipsis: more follows")
        sent.commands = []

        var shown: [Route] = []
        screens.performDropAction(.showSpeechModels) { shown.append($0) }
        XCTAssertEqual(shown, [.today])
        XCTAssertEqual(sent.commands, [], "nothing downloads from the Drop")
        XCTAssertEqual(screens.catalogue.speechDownload, .notStarted)
        XCTAssertEqual(screens.dictation.note, note, "the note stays, with its button")
        XCTAssertEqual(DropController.noteWithActionsDuration, .seconds(8), "long enough to be pressed")

        // What Today shows there: the line, the download with its size, and where it comes from.
        XCTAssertEqual(SpeechModels.todayLine(screens.catalogue.speech),
                       "No speech model yet, so nothing you say can be written down.")
        XCTAssertEqual(screens.catalogue.recommendedMB, 484)
        XCTAssertEqual(CatalogueModel.sources(screens.catalogue.models.filter { CatalogueModel.recommended.contains($0.id) }),
                       "huggingface.co and raw.githubusercontent.com")
    }

    /// Only what a missing model explains is said as one: typing that failed, or an edit's
    /// instruction not heard, keep their own words; a list that could not be read makes no claim.
    func testOnlyWhatAMissingModelExplainsIsSaidAsOne() throws {
        let none = SpeechModels(dictation: .missing, meetings: .missing)
        XCTAssertEqual(DictationModel.note(for: event(#"{"type":"dictation.failed","stage":"insert","message":"x"}"#), hasLanguageModel: false, speech: none)?.title,
                       "Couldn't type it here")
        XCTAssertEqual(DictationModel.note(for: event(#"{"type":"dictation.failed","stage":"other","message":"x"}"#), hasLanguageModel: false, speech: none)?.title,
                       "Dictation failed")
        XCTAssertEqual(DictationModel.note(for: event(#"{"type":"dictation.discarded","reason":"nothing_left"}"#), hasLanguageModel: false, speech: none)?.title,
                       "No speech model yet")
        XCTAssertEqual(DictationModel.note(for: event(#"{"type":"dictation.edit_failed","reason":"transcription"}"#), hasLanguageModel: false, speech: none)?.title,
                       "No speech model yet")

        let catalogue = CatalogueModel(send: { _ in })
        nothingInstalled(catalogue)
        catalogue.apply(event(#"{"type":"command.failed","command":"models.list","message":"the registry could not be read"}"#))
        XCTAssertEqual(catalogue.speech.dictation, .unknown)
    }

    func testLiveSaysTheMeetingIsRecordedButCannotBeTranscribed() {
        XCTAssertNil(SpeechModels.liveLine(.unknown))
        XCTAssertNil(SpeechModels.liveLine(SpeechModels(dictation: .missing, meetings: .served)))
        XCTAssertEqual(SpeechModels.liveLine(SpeechModels(dictation: .served, meetings: .missing)),
                       "No speech model is installed, so this meeting can't be transcribed. Its audio is recorded and kept with the record.")
        XCTAssertEqual(SpeechModels.ledgerEmptyLine(.unknown), "Waiting for someone to speak.")
        XCTAssertEqual(SpeechModels.ledgerEmptyLine(SpeechModels(dictation: .missing, meetings: .missing)),
                       "Nothing can be transcribed without a speech model.")
    }
}

private struct NoCalendar: CalendarAccess {
    func state() -> CardState { .notAsked }
    func request(done: @escaping @MainActor @Sendable () -> Void) {}
}
