// The theme engine: the user's appearance settings (kept in the core, appearance.*), what they
// resolve to now, and the mode applied to the app.
//
// The mode overrides the app's appearance (NSApp.appearance): Light and Dark pin it, Match system
// clears it so the app follows the system. Every token colour is dynamic, so the windows, the Drop
// and the menus follow at once. The dot colours depend on the mode shown and on the settings, so
// they are resolved here (GlowColours) and read by whatever draws them: the orb, the edge glow, the
// Drop, the transcript dots, the waveform, the Live legend and the menu-bar dot.
//
// Like every setting, each key is read with setting.get and changes with setting.set; the core's
// setting.value is the truth, and a key that could not be read or saved says so in Settings >
// Appearance, never as a silent default.
import AppKit
import InkBridge
import InkRenderer
import Observation
import SwiftUI

@MainActor
@Observable
final class GlowTheme {
    enum Mode: String, CaseIterable, Identifiable, Sendable {
        case light
        case dark
        case system

        var id: String { rawValue }

        var title: String {
            switch self {
            case .light: "Light"
            case .dark: "Dark"
            case .system: "Match system"
            }
        }
    }

    /// How the ink moves: as the system's Reduce Motion says, or always still.
    enum Motion: String, CaseIterable, Identifiable, Sendable {
        case system
        case still

        var id: String { rawValue }

        var title: String {
            switch self {
            case .system: "Follow system"
            case .still: "Always still"
            }
        }
    }

    /// What the core holds (each key's default until it says otherwise).
    struct Settings: Equatable, Sendable {
        var mode = Mode.system
        var dotsLight = "indigo"
        var dotsDark = "indigo"
        /// "#rrggbb", or nil for the preset's.
        var youLight: String?
        var themLight: String?
        var youDark: String?
        var themDark: String?
        var edgeGlow = true
        var motion = Motion.system
    }

    private(set) var settings = Settings()
    /// The system's (or the pinned) appearance is dark, as the app last drew.
    private(set) var effectiveDark = false
    /// Increase Contrast or Reduce Transparency is on: solid cards, a dimmed orb.
    private(set) var solidSurfaces = false
    /// A key could not be read or saved.
    private(set) var failure: String?

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private let applyAppearance: @MainActor (NSAppearance?) -> Void
    @ObservationIgnored private var effectiveObservation: NSKeyValueObservation?
    @ObservationIgnored private var displayOptions: NSObjectProtocol?
    /// The values written and not yet echoed, per key, oldest first. While a newer write is on
    /// its way, an older echo (setting.value answers every setting.set) is not applied: dragging
    /// in the colour panel writes many values, and the control must not jump back through them.
    @ObservationIgnored private var inFlight: [ShellSetting: [String]] = [:]

    /// `applyAppearance` sets the app's appearance (tests leave NSApp alone).
    init(send: @escaping SendCommand, applyAppearance: @escaping @MainActor (NSAppearance?) -> Void = { NSApp?.appearance = $0 }) {
        self.send = send
        self.applyAppearance = applyAppearance
    }

    /// The keys this engine reads and writes.
    static let keys: [ShellSetting] = [
        .appearanceMode, .appearanceDotsLight, .appearanceDotsDark, .appearanceYouLight, .appearanceThemLight,
        .appearanceYouDark, .appearanceThemDark, .appearanceEdgeGlow, .appearanceMotion,
    ]

    /// The ids of its setting commands (a command.failed carries one).
    static let settingIDs = Set(keys.map { "setting:\($0.rawValue)" })

    /// Follows the system's appearance and accessibility settings from now on (the app calls it once
    /// it has launched).
    func start() {
        guard effectiveObservation == nil, let app = NSApp else { return }
        effectiveDark = Self.isDark(app.effectiveAppearance)
        effectiveObservation = app.observe(\.effectiveAppearance) { [weak self] _, _ in
            Task { @MainActor in
                guard let self, let app = NSApp else { return }
                let dark = Self.isDark(app.effectiveAppearance)
                if self.effectiveDark != dark { self.effectiveDark = dark }
            }
        }
        readDisplayOptions()
        displayOptions = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.readDisplayOptions() }
        }
    }

    private func readDisplayOptions() {
        let workspace = NSWorkspace.shared
        let solid = workspace.accessibilityDisplayShouldIncreaseContrast
            || workspace.accessibilityDisplayShouldReduceTransparency
        if solid != solidSurfaces { solidSurfaces = solid }
    }

    nonisolated static func isDark(_ appearance: NSAppearance) -> Bool {
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
    }

    func load() {
        for key in Self.keys {
            send(.settingGet(key))
        }
    }

    // MARK: What it resolves to

    /// Whether the app shows its dark mode now.
    var isDark: Bool {
        switch settings.mode {
        case .light: false
        case .dark: true
        case .system: effectiveDark
        }
    }

    /// The preset chosen for the mode shown.
    var preset: Glow.Preset { Glow.preset(isDark ? settings.dotsDark : settings.dotsLight) }

    /// The user's own colours for the mode shown (nil: the preset's).
    var customYou: String? { isDark ? settings.youDark : settings.youLight }
    var customThem: String? { isDark ? settings.themDark : settings.themLight }

    /// Yours and theirs, as the mode shows them (fitted).
    var dots: (you: GlowColours.RGB, them: GlowColours.RGB) {
        GlowColours.dots(preset: preset, you: customYou, them: customThem, dark: isDark)
    }

    var you: Color { GlowColours.color(dots.you) }
    var them: Color { GlowColours.color(dots.them) }

    /// The orb's and the edge glow's colours.
    var palette: OrbPalette {
        GlowColours.palette(preset: preset, you: customYou, them: customThem, dark: isDark)
    }

    /// The ink holds still: the user's "Always still" (Reduce Motion is the renderer's own check).
    var motionStill: Bool { settings.motion == .still }

    // MARK: Changes

    func setMode(_ mode: Mode) {
        failure = nil
        settings.mode = mode
        applyMode()
        write(.appearanceMode, mode.rawValue)
    }

    /// Picks a preset for the mode shown, and drops that mode's own colours (the preset is the
    /// choice now).
    func setPreset(_ id: String) {
        failure = nil
        if isDark {
            settings.dotsDark = id
            settings.youDark = nil
            settings.themDark = nil
            write(.appearanceDotsDark, id)
            write(.appearanceYouDark, "preset")
            write(.appearanceThemDark, "preset")
        } else {
            settings.dotsLight = id
            settings.youLight = nil
            settings.themLight = nil
            write(.appearanceDotsLight, id)
            write(.appearanceYouLight, "preset")
            write(.appearanceThemLight, "preset")
        }
    }

    /// Your colour in the mode shown: "#rrggbb", or nil for the preset's.
    func setYou(_ hex: String?) {
        set(you: true, hex)
    }

    /// Their colour in the mode shown.
    func setThem(_ hex: String?) {
        set(you: false, hex)
    }

    private func set(you: Bool, _ hex: String?) {
        let value = hex.flatMap { GlowColours.parse($0) == nil ? nil : $0 }
        failure = nil
        let key: ShellSetting
        switch (you, isDark) {
        case (true, true):
            guard settings.youDark != value else { return }
            settings.youDark = value
            key = .appearanceYouDark
        case (true, false):
            guard settings.youLight != value else { return }
            settings.youLight = value
            key = .appearanceYouLight
        case (false, true):
            guard settings.themDark != value else { return }
            settings.themDark = value
            key = .appearanceThemDark
        case (false, false):
            guard settings.themLight != value else { return }
            settings.themLight = value
            key = .appearanceThemLight
        }
        write(key, value ?? "preset")
    }

    func setEdgeGlow(_ on: Bool) {
        failure = nil
        settings.edgeGlow = on
        write(.appearanceEdgeGlow, on ? "on" : "off")
    }

    func setMotion(_ motion: Motion) {
        failure = nil
        settings.motion = motion
        write(.appearanceMotion, motion.rawValue)
    }

    private func write(_ key: ShellSetting, _ value: String) {
        inFlight[key, default: []].append(value)
        send(.settingSet(key, value))
    }

    private func applyMode() {
        switch settings.mode {
        case .light: applyAppearance(NSAppearance(named: .aqua))
        case .dark: applyAppearance(NSAppearance(named: .darkAqua))
        case .system: applyAppearance(nil)
        }
    }

    // MARK: The core's answers

    func apply(_ event: InkEvent) {
        switch event {
        case .settingValue(let value):
            guard let key = ShellSetting(rawValue: value.key), Self.keys.contains(key) else { return }
            if var sent = inFlight[key], let echoed = value.value, let index = sent.firstIndex(of: echoed) {
                sent.removeSubrange(...index)
                inFlight[key] = sent.isEmpty ? nil : sent
                // A newer write is still on its way: this echo is out of date.
                if !sent.isEmpty { return }
            }
            take(key, value.value)
        case .commandFailed(let failed) where Self.settingIDs.contains(failed.id ?? ""):
            // What was written is in doubt: the next value the core reports is taken as it is.
            if let key = ShellSetting(rawValue: String((failed.id ?? "").dropFirst("setting:".count))) {
                inFlight[key] = nil
            }
            // The control keeps what it showed; the section says it may not be so.
            failure = failed.command == "setting.get"
                ? "Couldn't read your appearance settings, so Inkwell shows its defaults."
                : "Couldn't save that appearance setting: \(failed.message)"
        default:
            break
        }
    }

    /// One key's value from the core; an unset or unreadable value is the key's default.
    private func take(_ key: ShellSetting, _ value: String?) {
        var next = settings
        func dots(_ v: String?) -> String { Glow.presets.contains { $0.id == v } ? v! : Glow.presets[0].id }
        func colour(_ v: String?) -> String? { GlowColours.parse(v) == nil ? nil : v }
        switch key {
        case .appearanceMode: next.mode = value.flatMap(Mode.init(rawValue:)) ?? .system
        case .appearanceDotsLight: next.dotsLight = dots(value)
        case .appearanceDotsDark: next.dotsDark = dots(value)
        case .appearanceYouLight: next.youLight = colour(value)
        case .appearanceThemLight: next.themLight = colour(value)
        case .appearanceYouDark: next.youDark = colour(value)
        case .appearanceThemDark: next.themDark = colour(value)
        case .appearanceEdgeGlow: next.edgeGlow = value != "off"
        case .appearanceMotion: next.motion = value.flatMap(Motion.init(rawValue:)) ?? .system
        default: return
        }
        let modeChanged = next.mode != settings.mode
        if next != settings { settings = next }
        if modeChanged || key == .appearanceMode { applyMode() }
    }
}
