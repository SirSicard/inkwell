// A dictation key as the Mac writes it, and the keys the shortcut recorder can name.
//
// The core's token is the key (architecture rule 1: the core judges it, through hotkey.check);
// this file only shows a token in Mac notation (⌃⇧Space, Right ⌥, fn, F13) with a name VoiceOver
// reads ("Control-Shift-Space"), and names a pressed key for the recorder by its keyCode. Letters,
// digits and punctuation are named by position on an ANSI keyboard, as the core watches them
// (ink-platform-mac's binding.rs), so what is recorded is what the tap matches; they are shown as
// the user's own keyboard layout labels that key (display(_:)), so an AZERTY user who presses A
// sees A, although the token says q.
import Carbon.HIToolbox
import Foundation

enum KeyNotation {
    /// How a token reads, with the user's keyboard layout naming the keys that type a character.
    @MainActor
    static func display(_ token: String) -> DictationKey {
        describe(token, layout: KeyboardLayout.character(forKeyCode:))
    }

    /// How a token reads: its spoken name and its key cap. `layout` names a typing key by what the
    /// keyboard layout puts on it (nil: by its ANSI position). A token outside the core's grammar
    /// reads as itself.
    static func describe(_ token: String, layout: (Int) -> String? = { _ in nil }) -> DictationKey {
        if let alone = modifiersAlone[token] {
            return DictationKey(token: token, name: alone.name, cap: alone.cap)
        }
        let parts = token.split(separator: "+", omittingEmptySubsequences: false).map(String.init)
        // Modifiers with no key (recorded, for the core to refuse): their glyphs.
        if !parts.isEmpty, parts.allSatisfy({ chordModifiers[$0] != nil }) {
            return DictationKey(
                token: token,
                name: parts.compactMap { chordModifiers[$0]?.name }.joined(separator: "-"),
                cap: parts.compactMap { chordModifiers[$0]?.cap }.joined())
        }
        guard let last = parts.last, let ansi = keys[last] else {
            return DictationKey(token: token, name: token, cap: token)
        }
        let key = ansi.typed ? ansi.labelled(layout(ansi.keyCode)) : ansi
        var names: [String] = []
        var cap = ""
        var hasFn = false
        for part in parts.dropLast() {
            guard let modifier = chordModifiers[part] else {
                return DictationKey(token: token, name: token, cap: token)
            }
            if part == "fn" {
                hasFn = true
            } else {
                cap += modifier.cap
            }
            names.append(modifier.name)
        }
        names.append(key.name)
        // fn has no glyph of its own in a menu's shortcut column: it leads, spelt out.
        return DictationKey(
            token: token,
            name: names.joined(separator: "-"),
            cap: (hasFn ? "fn " : "") + cap + key.cap)
    }

    /// The core's tokens for modifiers held on their own, and the left-hand and Caps Lock tokens
    /// the recorder makes (the core refuses those, and says why).
    static let modifiersAlone: [String: (name: String, cap: String)] = [
        "fn": ("fn (Globe)", "fn"),
        "right_option": ("Right Option", "Right \u{2325}"),
        "right_command": ("Right Command", "Right \u{2318}"),
        "right_control": ("Right Control", "Right \u{2303}"),
        "right_shift": ("Right Shift", "Right \u{21E7}"),
        "left_option": ("Left Option", "Left \u{2325}"),
        "left_command": ("Left Command", "Left \u{2318}"),
        "left_control": ("Left Control", "Left \u{2303}"),
        "left_shift": ("Left Shift", "Left \u{21E7}"),
        "caps_lock": ("Caps Lock", "\u{21EA}"),
    ]

    /// A chord's modifiers, in the order the core's canonical token spells them.
    static let chordOrder = ["fn", "ctrl", "option", "shift", "cmd"]

    static let chordModifiers: [String: (name: String, cap: String)] = [
        "fn": ("fn", "fn"),
        "ctrl": ("Control", "\u{2303}"),
        "option": ("Option", "\u{2325}"),
        "shift": ("Shift", "\u{21E7}"),
        "cmd": ("Command", "\u{2318}"),
    ]

    /// Each key the core names (its token), how it reads, and its keyCode.
    struct Key {
        let token: String
        let name: String
        let cap: String
        let keyCode: Int
        /// Laptops set the Fn flag on these keys by themselves (the function row, the arrows and
        /// the keys Fn+arrow makes): the recorder does not count it as part of the shortcut.
        var setsFn = false
        /// It types a character, which the keyboard layout may label otherwise.
        var typed = false

        /// This key as the layout labels it: `label`, when it is one visible character other than
        /// its ANSI one, as both cap and name.
        func labelled(_ label: String?) -> Key {
            guard let label, label.count == 1,
                  label.unicodeScalars.allSatisfy({ !CharacterSet.whitespacesAndNewlines.union(.controlCharacters).contains($0) })
            else { return self }
            let shown = label.uppercased()
            guard shown != cap else { return self }
            return Key(token: token, name: shown, cap: shown, keyCode: keyCode, setsFn: setsFn, typed: typed)
        }
    }

    static let allKeys: [Key] = {
        let letters: [(String, Int)] = [
            ("a", kVK_ANSI_A), ("b", kVK_ANSI_B), ("c", kVK_ANSI_C), ("d", kVK_ANSI_D), ("e", kVK_ANSI_E),
            ("f", kVK_ANSI_F), ("g", kVK_ANSI_G), ("h", kVK_ANSI_H), ("i", kVK_ANSI_I), ("j", kVK_ANSI_J),
            ("k", kVK_ANSI_K), ("l", kVK_ANSI_L), ("m", kVK_ANSI_M), ("n", kVK_ANSI_N), ("o", kVK_ANSI_O),
            ("p", kVK_ANSI_P), ("q", kVK_ANSI_Q), ("r", kVK_ANSI_R), ("s", kVK_ANSI_S), ("t", kVK_ANSI_T),
            ("u", kVK_ANSI_U), ("v", kVK_ANSI_V), ("w", kVK_ANSI_W), ("x", kVK_ANSI_X), ("y", kVK_ANSI_Y),
            ("z", kVK_ANSI_Z),
        ]
        let digits: [(String, Int)] = [
            ("0", kVK_ANSI_0), ("1", kVK_ANSI_1), ("2", kVK_ANSI_2), ("3", kVK_ANSI_3), ("4", kVK_ANSI_4),
            ("5", kVK_ANSI_5), ("6", kVK_ANSI_6), ("7", kVK_ANSI_7), ("8", kVK_ANSI_8), ("9", kVK_ANSI_9),
        ]
        let functionKeys = [
            kVK_F1, kVK_F2, kVK_F3, kVK_F4, kVK_F5, kVK_F6, kVK_F7, kVK_F8, kVK_F9, kVK_F10,
            kVK_F11, kVK_F12, kVK_F13, kVK_F14, kVK_F15, kVK_F16, kVK_F17, kVK_F18, kVK_F19, kVK_F20,
        ]
        var all = letters.map { Key(token: $0.0, name: $0.0.uppercased(), cap: $0.0.uppercased(), keyCode: $0.1, typed: true) }
        all += digits.map { Key(token: $0.0, name: $0.0, cap: $0.0, keyCode: $0.1, typed: true) }
        all += functionKeys.enumerated().map { i, code in
            Key(token: "f\(i + 1)", name: "F\(i + 1)", cap: "F\(i + 1)", keyCode: code, setsFn: true)
        }
        all += [
            Key(token: "space", name: "Space", cap: "Space", keyCode: kVK_Space),
            Key(token: "return", name: "Return", cap: "\u{21A9}", keyCode: kVK_Return),
            Key(token: "tab", name: "Tab", cap: "\u{21E5}", keyCode: kVK_Tab),
            Key(token: "escape", name: "Escape", cap: "\u{238B}", keyCode: kVK_Escape),
            Key(token: "delete", name: "Delete", cap: "\u{232B}", keyCode: kVK_Delete),
            Key(token: "forward_delete", name: "Forward Delete", cap: "\u{2326}", keyCode: kVK_ForwardDelete, setsFn: true),
            Key(token: "left", name: "Left Arrow", cap: "\u{2190}", keyCode: kVK_LeftArrow, setsFn: true),
            Key(token: "right", name: "Right Arrow", cap: "\u{2192}", keyCode: kVK_RightArrow, setsFn: true),
            Key(token: "down", name: "Down Arrow", cap: "\u{2193}", keyCode: kVK_DownArrow, setsFn: true),
            Key(token: "up", name: "Up Arrow", cap: "\u{2191}", keyCode: kVK_UpArrow, setsFn: true),
            Key(token: "home", name: "Home", cap: "\u{2196}", keyCode: kVK_Home, setsFn: true),
            Key(token: "end", name: "End", cap: "\u{2198}", keyCode: kVK_End, setsFn: true),
            Key(token: "page_up", name: "Page Up", cap: "\u{21DE}", keyCode: kVK_PageUp, setsFn: true),
            Key(token: "page_down", name: "Page Down", cap: "\u{21DF}", keyCode: kVK_PageDown, setsFn: true),
            Key(token: "minus", name: "Minus", cap: "-", keyCode: kVK_ANSI_Minus, typed: true),
            Key(token: "equal", name: "Equals", cap: "=", keyCode: kVK_ANSI_Equal, typed: true),
            Key(token: "left_bracket", name: "Left Bracket", cap: "[", keyCode: kVK_ANSI_LeftBracket, typed: true),
            Key(token: "right_bracket", name: "Right Bracket", cap: "]", keyCode: kVK_ANSI_RightBracket, typed: true),
            Key(token: "backslash", name: "Backslash", cap: "\\", keyCode: kVK_ANSI_Backslash, typed: true),
            Key(token: "semicolon", name: "Semicolon", cap: ";", keyCode: kVK_ANSI_Semicolon, typed: true),
            Key(token: "quote", name: "Quote", cap: "'", keyCode: kVK_ANSI_Quote, typed: true),
            Key(token: "comma", name: "Comma", cap: ",", keyCode: kVK_ANSI_Comma, typed: true),
            Key(token: "period", name: "Period", cap: ".", keyCode: kVK_ANSI_Period, typed: true),
            Key(token: "slash", name: "Slash", cap: "/", keyCode: kVK_ANSI_Slash, typed: true),
            Key(token: "grave", name: "Grave Accent", cap: "`", keyCode: kVK_ANSI_Grave, typed: true),
            // The key left of 1 on ISO keyboards.
            Key(token: "section", name: "Section", cap: "\u{00A7}", keyCode: kVK_ISO_Section, typed: true),
        ]
        return all
    }()

    /// By token.
    static let keys: [String: Key] = Dictionary(uniqueKeysWithValues: allKeys.map { ($0.token, $0) })
    /// By keyCode.
    static let byKeyCode: [Int: Key] = Dictionary(uniqueKeysWithValues: allKeys.map { ($0.keyCode, $0) })

    /// What a shortcut the app knows macOS or most apps take would clash with, said once it is
    /// saved. Known ones only: macOS has many more, and the user's own apps more still. `cap` is
    /// how the shortcut is shown (display(_:)): shortcuts follow the characters on the keys, so
    /// ⌘Q is Quit wherever the layout puts Q, and the match is by what is shown, not by position.
    static func clash(_ token: String, cap: String? = nil) -> String? {
        let cap = cap ?? describe(token).cap
        let match = { (list: [String: String]) in list.first { describe($0.key).cap == cap }?.value }
        if let what = match(systemShortcuts) {
            return "\(cap) is the shortcut for \(what), so the two may clash."
        }
        if let what = match(appShortcuts) {
            return "\(cap) is \(what) in most apps; while dictation is on, Inkwell takes it from them."
        }
        return nil
    }

    /// macOS's own, as a new Mac sets them.
    static let systemShortcuts: [String: String] = [
        "cmd+space": "Spotlight",
        "option+cmd+space": "a Finder search window",
        "ctrl+space": "switching input sources",
        "ctrl+option+space": "the next input source",
        "ctrl+cmd+space": "the Character Viewer",
        "cmd+tab": "switching apps",
        "shift+cmd+3": "a screenshot",
        "shift+cmd+4": "a screenshot of a selection",
        "shift+cmd+5": "the Screenshot app",
        "ctrl+up": "Mission Control",
        "ctrl+down": "App Exposé",
        "ctrl+left": "moving to the Space on the left",
        "ctrl+right": "moving to the Space on the right",
        "ctrl+cmd+q": "locking the screen",
        "shift+cmd+q": "logging out",
        "option+cmd+escape": "Force Quit",
    ]

    /// Shortcuts nearly every app gives the same meaning.
    static let appShortcuts: [String: String] = [
        "cmd+c": "Copy", "cmd+v": "Paste", "cmd+x": "Cut", "cmd+z": "Undo", "shift+cmd+z": "Redo",
        "cmd+a": "Select All", "cmd+s": "Save", "cmd+w": "Close Window", "cmd+q": "Quit",
        "cmd+h": "Hide", "cmd+m": "Minimize", "cmd+n": "New", "cmd+o": "Open", "cmd+p": "Print",
        "cmd+f": "Find", "cmd+t": "New Tab", "cmd+comma": "Settings",
    ]
}

/// The user's keyboard layout, for showing a key as it is labelled. Text Input Sources asserts it
/// runs on the main thread, hence the isolation.
@MainActor
enum KeyboardLayout {
    /// What the key at `keyCode` types with no modifier in the current layout, or nil.
    static func character(forKeyCode keyCode: Int) -> String? {
        guard let source = TISCopyCurrentKeyboardLayoutInputSource()?.takeRetainedValue(),
              let raw = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData)
        else { return nil }
        let data = Unmanaged<CFData>.fromOpaque(raw).takeUnretainedValue() as Data
        var deadKeys: UInt32 = 0
        var length = 0
        var chars = [UniChar](repeating: 0, count: 4)
        let status = data.withUnsafeBytes { buffer -> OSStatus in
            guard let layout = buffer.baseAddress?.assumingMemoryBound(to: UCKeyboardLayout.self) else {
                return OSStatus(paramErr)
            }
            return UCKeyTranslate(
                layout, UInt16(keyCode), UInt16(kUCKeyActionDisplay), 0, UInt32(LMGetKbdType()),
                OptionBits(1 << kUCKeyTranslateNoDeadKeysBit), &deadKeys, chars.count, &length, &chars)
        }
        guard status == noErr, length > 0 else { return nil }
        return String(utf16CodeUnits: chars, count: length)
    }
}
