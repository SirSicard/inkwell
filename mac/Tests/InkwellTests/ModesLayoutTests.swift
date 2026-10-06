// Settings > Modes and its editor in the app's window at its 720-pt minimum (and wider): the
// editor's sheet fits over the window, and every control it draws lies inside its padding, with
// the widest content it can hold (a long name, a model with a long name at a cloud provider, a
// confirm, many apps, apps moving from another mode, an error). The rows themselves are in
// SettingsCardsLayoutTests' page, which lists modes like these.
import AppKit
import InkBridge
import SwiftUI
import XCTest

@testable import Inkwell

private func event(_ json: String, file: StaticString = #filePath, line: UInt = #line) -> InkEvent {
    guard let decoded = try? InkEvent.decode(Data(json.utf8)) else {
        XCTFail("not an event: \(json)", file: file, line: line)
        return .unknown(type: "")
    }
    return decoded
}

private struct NoCalendar: CalendarAccess {
    func state() -> CardState { .notAsked }
    func request(done: @escaping @MainActor @Sendable () -> Void) {}
}

private struct ManyRunning: RunningApps {
    func running() -> [PickedApp] {
        ["com.apple.mail", "com.apple.Notes", "com.tinyspeck.slackmacgap"].map {
            PickedApp(identity: $0, name: AppIdentity.label($0, apps: WorkspaceApps()).name, icon: nil)
        }
    }
}

/// Modes whose rows are as wide and as tall as rows get: a long name, many apps, a model that
/// moved (Confirm…), one missing, one needing an OK (Allow…). Used by SettingsCardsLayoutTests too.
enum WideModes {
    static let apps = [
        "com.apple.mail", "com.apple.Notes", "com.tinyspeck.slackmacgap", "net.whatsapp.WhatsApp",
        "com.microsoft.VSCode", "com.microsoft.teams2", "com.microsoft.Outlook", "com.google.Chrome",
        "com.googlecode.iterm2", "ru.keepcoder.Telegram",
    ]

    static var listing: String {
        let apps = String(decoding: (try? JSONSerialization.data(withJSONObject: apps)) ?? Data(), as: UTF8.self)
        let modes = [
            #"{"id":"w","name":"Writing for the team channel, carefully","style":"relaxed","polish":true,"remove_fillers":true,"polish_prompt":"","apps":\#(apps),"polish_model":"provider:openrouter","polish_model_name":"a-provider/a-model-with-a-very-long-name-indeed","polish_model_state":"moved"}"#,
            #"{"id":"m","name":"Mail","style":"formal","polish":true,"remove_fillers":false,"polish_prompt":"Sign off as me.","apps":["com.apple.mail"],"polish_model":"provider:anthropic","polish_model_state":"missing"}"#,
            #"{"id":"k","name":"Chat","style":"casual","polish":true,"remove_fillers":true,"polish_prompt":"","apps":["com.tinyspeck.slackmacgap"],"polish_model":"provider:openrouter","polish_model_state":"ready"}"#,
            #"{"id":"n","name":"Notes","style":"other","polish":false,"remove_fillers":true,"polish_prompt":"","apps":[]}"#,
            #"{"id":"d","name":"Default","style":"formal","polish":true,"remove_fillers":true,"polish_prompt":"","apps":[]}"#,
        ]
        let models = [
            #"{"id":"engine:apple-foundation-models","name":"SystemLanguageModel.default","model":"SystemLanguageModel.default","to":"on_device","allowed":true,"blocked_local_only":false}"#,
            #"{"id":"provider:openrouter","name":"OpenRouter","model":"another-provider/the-default-model-chosen-in-ai","to":"cloud","endpoint":"https://openrouter.ai/api/v1","allowed":false,"blocked_local_only":false}"#,
        ]
        return #"{"type":"modes.listed","default_id":"d","default_polish_prompt":"Fix the punctuation and the obvious slips of the tongue. Keep the speaker's words and meaning; never add anything they did not say.","modes":[\#(modes.joined(separator: ","))],"polish_models":[\#(models.joined(separator: ","))],"setting_polish_model":"engine:apple-foundation-models"}"#
    }

    /// Polish on, with an OK on this Mac and one for a cloud provider (Settings > AI's list).
    static let consents = #"{"type":"consent.state","feature":"polish","on":true,"allowed":true,"to":"on_device","name":"SystemLanguageModel.default","consents":[{"to":"on_device"},{"to":"cloud","name":"OpenRouter","endpoint":"https://openrouter.ai/api/v1"}]}"#
}

@MainActor
final class ModesLayoutTests: XCTestCase {
    private final class NoEvents: UpcomingEvents {
        func nextEvent(after now: Date, within horizon: TimeInterval) -> UpcomingEvent? { nil }
    }

    private func screens() -> ScreenModels {
        let screens = ScreenModels(send: { _ in }, calendar: NoCalendar(), apps: WorkspaceApps(), runningApps: ManyRunning())
        screens.polish.apply(event(WideModes.consents))
        screens.modes.apply(event(WideModes.listing))
        return screens
    }

    /// The widest editor: the long-named mode, its moved model, every app, one more taken from
    /// another mode, and an error.
    private func openWidest(_ screens: ScreenModels) throws -> ModeEditor {
        screens.modes.edit("w")
        let editor = try XCTUnwrap(screens.modes.editor)
        editor.polishModelName = "a-provider/a-model-with-a-very-long-name-indeed"
        editor.prompt = String(repeating: "Keep it short and plain. ", count: 12)
        screens.modes.addApp("com.example.not-installed-anywhere", to: editor)
        editor.error = ModesModel.saveFailure(.listUnreadable, editor: editor)
        return editor
    }

    private func settle(_ window: NSWindow, until done: () -> Bool = { false }) {
        let deadline = Date().addingTimeInterval(3)
        var passes = 0
        repeat {
            window.contentViewController?.view.layoutSubtreeIfNeeded()
            window.attachedSheet?.contentView?.layoutSubtreeIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
            passes += 1
        } while (passes < 6 || !done()) && Date() < deadline
    }

    private func descendants<T: NSView>(of view: NSView, as type: T.Type) -> [T] {
        view.subviews.flatMap { ([$0 as? T].compactMap { $0 }) + descendants(of: $0, as: type) }
    }

    /// The editor opened from Settings over the window at 720 by 460: a sheet no wider than the
    /// window, its size, with every control inside its padding.
    func testTheEditorSheetFitsOverTheWindowAtItsMinimum() throws {
        let screens = screens()
        let store = CoreStore()
        let router = Router()
        router.open(.settings)
        let root = ShellView(router: router).environment(store).environment(ShellInk(store: store))
            .environment(Updates(infoDictionary: nil)).environment(screens).environment(LibraryModel(send: { _ in }))
            .environment(UpNextModel(access: NoCalendar(), events: NoEvents())).environment(router)
            .environment(WindowPresence()).environment(screens.theme).tint(Theme.buttonFill)
        let window = MainWindowController.makeWindow(root: root)
        defer { window.close() }
        window.setContentSize(MainWindowController.minimumContentSize)
        window.orderFront(nil)
        settle(window)
        _ = try openWidest(screens)
        settle(window) { window.attachedSheet != nil }
        let sheet = try XCTUnwrap(window.attachedSheet, "the editor is a sheet on the window")
        settle(window)
        XCTAssertEqual(window.frame.width, MainWindowController.minimumContentSize.width, accuracy: 0.5, "the sheet widened the window")
        XCTAssertLessThanOrEqual(sheet.frame.width, window.frame.width, "the sheet is wider than the window")
        let content = try XCTUnwrap(sheet.contentView)
        XCTAssertEqual(content.frame.width, ModeEditorSheet.size.width, accuracy: 0.5)
        XCTAssertEqual(content.frame.height, ModeEditorSheet.size.height, accuracy: 0.5)
        try checkControls(in: content, sheet: content.bounds, label: "over the window")
        if let folder = ProcessInfo.processInfo.environment["INK_SETTINGS_RENDER"] {
            let out = URL(fileURLWithPath: folder, isDirectory: true)
            try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
            try draw(content, to: out.appendingPathComponent("modes-editor-720.png"))
        }
    }

    /// The editor on its own, in Light and Dark and for a new mode and the default one: the same.
    func testEveryEditorKeepsItsControlsInsideItsPadding() throws {
        let screens = screens()
        defer { NSApp.appearance = nil }
        let cases: [(String, @MainActor () throws -> ModeEditor)] = [
            ("widest", { try self.openWidest(screens) }),
            ("new", { screens.modes.add(); return try XCTUnwrap(screens.modes.editor) }),
            ("default", { screens.modes.edit("d"); return try XCTUnwrap(screens.modes.editor) }),
            ("own style", { screens.modes.edit("n"); return try XCTUnwrap(screens.modes.editor) }),
        ]
        for mode in [GlowTheme.Mode.light, .dark] {
            screens.theme.setMode(mode)
            for (name, open) in cases {
                screens.modes.closeEditor()
                let editor = try open()
                // At the window's top left, so a sheet whose content overflows it shows as a
                // control left of its padding or past its right edge.
                let window = MainWindowController.makeWindow(root: ModeEditorSheet(modes: screens.modes, editor: editor)
                    .followsAppMode().environment(screens.theme)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading))
                defer { window.close() }
                window.setContentSize(NSSize(width: 720, height: ModeEditorSheet.size.height))
                settle(window)
                let content = try XCTUnwrap(window.contentViewController?.view)
                try checkControls(in: content, sheet: CGRect(origin: .zero, size: ModeEditorSheet.size), label: "\(name), \(mode.rawValue)")
                if let folder = ProcessInfo.processInfo.environment["INK_SETTINGS_RENDER"] {
                    let out = URL(fileURLWithPath: folder, isDirectory: true)
                    try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
                    try draw(content, to: out.appendingPathComponent("modes-editor-\(name.replacingOccurrences(of: " ", with: "-"))-\(mode.rawValue).png"))
                }
            }
        }
    }

    /// Every AppKit control the editor draws, visible, lies inside its padding (a focus ring's
    /// point aside), and the editor holds the controls each row needs.
    /// `sheet`: the sheet's frame in `content`, from its top left.
    private func checkControls(in content: NSView, sheet: CGRect, label: String) throws {
        let controls = descendants(of: content, as: NSControl.self)
            .filter { !$0.isHiddenOrHasHiddenAncestor && $0.bounds.width > 0 && $0.bounds.height > 0 }
        XCTAssertGreaterThanOrEqual(controls.count, 5, "\(label): the editor's controls were found")
        let room = sheet.insetBy(dx: ModeEditorSheet.padding - 1, dy: ModeEditorSheet.padding - 1)
        for control in controls {
            let rect = control.convert(control.bounds, to: content)
            // From the top left, whichever way the content counts.
            let frame = content.isFlipped
                ? rect : CGRect(x: rect.minX, y: content.bounds.height - rect.maxY, width: rect.width, height: rect.height)
            // A control scrolled out of view is clipped by the scroll view, not drawn outside.
            guard let clip = control.enclosingScrollView, clip !== control else {
                XCTAssertTrue(room.contains(frame), "\(label): \(type(of: control)) \(control.accessibilityLabel() ?? "") at \(frame) is outside \(room)")
                continue
            }
            let visible = rect.intersection(clip.convert(clip.bounds, to: content))
            if visible.isNull || visible.isEmpty { continue }
            XCTAssertGreaterThanOrEqual(frame.minX, room.minX - 0.5, "\(label): \(type(of: control)) \(control.accessibilityLabel() ?? "") starts left of the padding")
            XCTAssertLessThanOrEqual(frame.maxX, room.maxX + 0.5, "\(label): \(type(of: control)) \(control.accessibilityLabel() ?? "") at \(frame) runs past the padding")
        }
        // The scroll view holds the fields: its document is never wider than it is.
        for scroll in descendants(of: content, as: NSScrollView.self) {
            guard let document = scroll.documentView else { continue }
            XCTAssertLessThanOrEqual(document.frame.width, scroll.contentView.bounds.width + 0.5, "\(label): the editor scrolls sideways")
        }
    }

    private func draw(_ view: NSView, to url: URL) throws {
        let rep = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: rep)
        try XCTUnwrap(rep.representation(using: .png, properties: [:])).write(to: url)
    }
}
