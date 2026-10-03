// A dictation key as the Mac writes it, and the keys the shortcut recorder can name.
//
// The core's token is the key (architecture rule 1: the core judges it, through hotkey.check);
// this file only shows a token in Mac notation (⌃⇧Space, Right ⌥, fn, F13) with a name VoiceOver
// reads ("Control-Shift-Space"), and names a pressed key for the recorder by its keyCode. Letters,
// digits and punctuation are named by position on an ANSI keyboard, as the core watches them
// (ink-platform-mac's binding.rs), so what is recorded is what the tap matches.
import Carbon.HIToolbox
import Foundation

enum KeyNotation {
    /// How a token reads: its spoken name and its key cap. A token outside the core's grammar reads
    /// as itself.
    static func describe(_ token: String) -> DictationKey {
        if let alone = modifiersAlone[token] {
            return DictationKey(token: token, name: alone.name, cap: alone.cap)
        }
        let parts = token.split(separator: "+", omittingEmptySubsequences: false).map(String.init)
        guard let last = parts.last, let key = keys[last] else {
            return DictationKey(token: token, name: token, cap: token)
        }
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
        var all = letters.map { Key(token: $0.0, name: $0.0.uppercased(), cap: $0.0.uppercased(), keyCode: $0.1) }
        all += digits.map { Key(token: $0.0, name: $0.0, cap: $0.0, keyCode: $0.1) }
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
            Key(token: "minus", name: "Minus", cap: "-", keyCode: kVK_ANSI_Minus),
            Key(token: "equal", name: "Equals", cap: "=", keyCode: kVK_ANSI_Equal),
            Key(token: "left_bracket", name: "Left Bracket", cap: "[", keyCode: kVK_ANSI_LeftBracket),
            Key(token: "right_bracket", name: "Right Bracket", cap: "]", keyCode: kVK_ANSI_RightBracket),
            Key(token: "backslash", name: "Backslash", cap: "\\", keyCode: kVK_ANSI_Backslash),
            Key(token: "semicolon", name: "Semicolon", cap: ";", keyCode: kVK_ANSI_Semicolon),
            Key(token: "quote", name: "Quote", cap: "'", keyCode: kVK_ANSI_Quote),
            Key(token: "comma", name: "Comma", cap: ",", keyCode: kVK_ANSI_Comma),
            Key(token: "period", name: "Period", cap: ".", keyCode: kVK_ANSI_Period),
            Key(token: "slash", name: "Slash", cap: "/", keyCode: kVK_ANSI_Slash),
            Key(token: "grave", name: "Grave Accent", cap: "`", keyCode: kVK_ANSI_Grave),
        ]
        return all
    }()

    /// By token.
    static let keys: [String: Key] = Dictionary(uniqueKeysWithValues: allKeys.map { ($0.token, $0) })
    /// By keyCode.
    static let byKeyCode: [Int: Key] = Dictionary(uniqueKeysWithValues: allKeys.map { ($0.keyCode, $0) })

    /// What a shortcut the app knows macOS or most apps take would clash with, said once it is
    /// saved. Known ones only: macOS has many more, and the user's own apps more still.
    static func clash(_ token: String) -> String? {
        let cap = describe(token).cap
        if let what = systemShortcuts[token] {
            return "\(cap) is the shortcut for \(what), so the two may clash."
        }
        if let what = appShortcuts[token] {
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
