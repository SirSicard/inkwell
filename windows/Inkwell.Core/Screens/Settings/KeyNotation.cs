// A dictation key as Windows writes it, and the keys the shortcut recorder can name. The Mac's
// KeyNotation, Windows' version.
//
// The core's token is the key (architecture rule 1: the core judges it, through hotkey.check);
// this file only shows a token in Windows notation (Ctrl+Shift+Space, Right Ctrl, F13) with a name
// Narrator reads, and names a pressed key for the recorder by its virtual key. Letters, digits and
// punctuation are the virtual keys Windows gives (the punctuation keys by their US position, as
// ink-platform-win's binding.rs names them), so what is recorded is what the hook matches; they are
// shown as the user's own keyboard layout labels that key (Display), so a user whose layout puts Ö
// on the semicolon key sees Ö.
namespace Inkwell.Core.Screens;

public static class KeyNotation
{
    /// <summary>One key the core names: its token, how it reads, its virtual key, and whether it types a character (the layout may label it otherwise).</summary>
    public sealed record Key(string Token, string Name, string Cap, uint Vk, bool Typed = false)
    {
        /// <summary>This key as the layout labels it: <paramref name="label"/>, when it is one visible character other than its US one, as both cap and name.</summary>
        public Key Labelled(string? label)
        {
            if (label is not { Length: 1 } || char.IsWhiteSpace(label[0]) || char.IsControl(label[0]))
            {
                return this;
            }
            var shown = label.ToUpperInvariant();
            return shown == Cap ? this : this with { Name = shown, Cap = shown };
        }
    }

    /// <summary>Every key the recorder can name, by token and by virtual key.</summary>
    public static IReadOnlyList<Key> AllKeys { get; } = MakeKeys();

    private static readonly Dictionary<string, Key> ByToken = AllKeys.ToDictionary(k => k.Token, StringComparer.Ordinal);
    private static readonly Dictionary<uint, Key> ByVk = AllKeys.ToDictionary(k => k.Vk);

    private static List<Key> MakeKeys()
    {
        var keys = new List<Key>();
        for (var c = 'a'; c <= 'z'; c++)
        {
            keys.Add(new(c.ToString(), char.ToUpperInvariant(c).ToString(), char.ToUpperInvariant(c).ToString(), char.ToUpperInvariant(c), Typed: true));
        }
        for (var c = '0'; c <= '9'; c++)
        {
            keys.Add(new(c.ToString(), c.ToString(), c.ToString(), c, Typed: true));
        }
        for (var n = 1; n <= 24; n++)
        {
            keys.Add(new($"f{n}", $"F{n}", $"F{n}", (uint)(0x70 + n - 1)));
        }
        keys.AddRange(
        [
            new("space", "Space", "Space", 0x20),
            new("return", "Enter", "Enter", 0x0D),
            new("tab", "Tab", "Tab", 0x09),
            new("escape", "Esc", "Esc", 0x1B),
            // The Mac's names: "delete" is the key left of the backspace arrow (Backspace on a PC),
            // "forward_delete" is Delete.
            new("delete", "Backspace", "Backspace", 0x08),
            new("forward_delete", "Delete", "Delete", 0x2E),
            new("left", "Left Arrow", "Left", 0x25),
            new("right", "Right Arrow", "Right", 0x27),
            new("down", "Down Arrow", "Down", 0x28),
            new("up", "Up Arrow", "Up", 0x26),
            new("home", "Home", "Home", 0x24),
            new("end", "End", "End", 0x23),
            new("page_up", "Page Up", "Page Up", 0x21),
            new("page_down", "Page Down", "Page Down", 0x22),
            new("minus", "Minus", "-", 0xBD, Typed: true),
            new("equal", "Equals", "=", 0xBB, Typed: true),
            new("left_bracket", "Left Bracket", "[", 0xDB, Typed: true),
            new("right_bracket", "Right Bracket", "]", 0xDD, Typed: true),
            new("backslash", "Backslash", "\\", 0xDC, Typed: true),
            new("semicolon", "Semicolon", ";", 0xBA, Typed: true),
            new("quote", "Quote", "'", 0xDE, Typed: true),
            new("comma", "Comma", ",", 0xBC, Typed: true),
            new("period", "Period", ".", 0xBE, Typed: true),
            new("slash", "Slash", "/", 0xBF, Typed: true),
            new("grave", "Grave Accent", "`", 0xC0, Typed: true),
            // The extra key by left Shift on ISO keyboards.
            new("oem_102", "Less Than", "<", 0xE2, Typed: true),
        ]);
        return keys;
    }

    /// <summary>The key at <paramref name="vk"/>, if the core names it.</summary>
    public static Key? ForVk(uint vk) => ByVk.GetValueOrDefault(vk);

    /// <summary>
    /// The core's tokens for modifiers held on their own, and the left-hand and Caps Lock tokens the
    /// recorder makes (the core refuses those, and says why); the Mac's names where a library came
    /// from one.
    /// </summary>
    private static readonly Dictionary<string, (string Name, string Cap)> ModifiersAlone = new(StringComparer.Ordinal)
    {
        ["right_control"] = ("Right Ctrl", "Right Ctrl"),
        ["right_alt"] = ("Right Alt", "Right Alt"),
        ["right_shift"] = ("Right Shift", "Right Shift"),
        ["right_win"] = ("Right Windows key", "Right Win"),
        ["left_control"] = ("Left Ctrl", "Left Ctrl"),
        ["left_alt"] = ("Left Alt", "Left Alt"),
        ["left_shift"] = ("Left Shift", "Left Shift"),
        ["left_win"] = ("Left Windows key", "Left Win"),
        ["caps_lock"] = ("Caps Lock", "Caps Lock"),
        ["fn"] = ("Fn", "Fn"),
        ["right_option"] = ("Right Alt", "Right Alt"),
        ["right_command"] = ("Right Command", "Right Command"),
    };

    /// <summary>A chord's modifiers, in the order the core's canonical token spells them.</summary>
    public static IReadOnlyList<string> ChordOrder { get; } = ["ctrl", "alt", "shift", "win"];

    private static readonly Dictionary<string, string> ChordModifiers = new(StringComparer.Ordinal)
    {
        ["ctrl"] = "Ctrl",
        ["alt"] = "Alt",
        ["shift"] = "Shift",
        ["win"] = "Win",
    };

    /// <summary>
    /// The keyboard layout every key name uses when none is passed (the status line, Today, the
    /// first run, the recorder alike): the app sets it to <see cref="KeyboardLayout.Character"/>
    /// at start. Null (tests): typing keys by their US position.
    /// </summary>
    public static Func<uint, string?>? Layout { get; set; }

    /// <summary>
    /// How a token reads: its spoken name and its key cap ("Ctrl+Shift+Space", "Right Ctrl").
    /// <paramref name="layout"/> names a typing key by what the keyboard layout puts on it (null:
    /// by its US position). A token outside the core's grammar reads as itself.
    /// </summary>
    public static DictationKey Describe(string token, Func<uint, string?>? layout = null)
    {
        ArgumentNullException.ThrowIfNull(token);
        layout ??= Layout;
        if (ModifiersAlone.TryGetValue(token, out var alone))
        {
            return new DictationKey(token, alone.Name, alone.Cap);
        }
        var parts = token.Split('+');
        // Modifiers with no key (recorded, for the core to refuse): their names.
        if (parts.All(ChordModifiers.ContainsKey))
        {
            var joined = string.Join("+", parts.Select(p => ChordModifiers[p]));
            return new DictationKey(token, joined, joined);
        }
        if (!ByToken.TryGetValue(parts[^1], out var key))
        {
            return new DictationKey(token, token, token);
        }
        if (key.Typed && layout is not null)
        {
            key = key.Labelled(layout(key.Vk));
        }
        var names = new List<string>();
        var caps = new List<string>();
        foreach (var part in parts[..^1])
        {
            if (!ChordModifiers.TryGetValue(part, out var modifier))
            {
                return new DictationKey(token, token, token);
            }
            names.Add(modifier);
            caps.Add(modifier);
        }
        names.Add(key.Name);
        caps.Add(key.Cap);
        return new DictationKey(token, string.Join("+", names), string.Join("+", caps));
    }

    /// <summary>
    /// What a shortcut Windows or most apps take would clash with, said once it is saved. Known
    /// ones only: Windows has many more, and the user's own apps more still. <paramref name="cap"/>
    /// is how the shortcut is shown (Describe): shortcuts follow the characters on the keys, so
    /// Ctrl+Z is Undo wherever the layout puts Z, and the match is by what is shown.
    /// </summary>
    public static string? Clash(string token, string? cap = null)
    {
        ArgumentNullException.ThrowIfNull(token);
        cap ??= Describe(token).Cap;
        string? Match(IReadOnlyDictionary<string, string> list) =>
            list.FirstOrDefault(entry => Describe(entry.Key).Cap == cap).Value;
        if (Match(SystemShortcuts) is string system)
        {
            return $"{cap} is the shortcut for {system}, so the two may clash.";
        }
        if (Match(AppShortcuts) is string app)
        {
            return $"{cap} is {app} in most apps; while dictation is on, Inkwell takes it from them.";
        }
        return null;
    }

    /// <summary>Windows' own, as a new PC sets them (a few; Ctrl+Alt+Del and Win+L never reach an app at all).</summary>
    public static IReadOnlyDictionary<string, string> SystemShortcuts { get; } = new Dictionary<string, string>(StringComparer.Ordinal)
    {
        ["win+l"] = "locking the PC",
        ["ctrl+alt+forward_delete"] = "the Ctrl+Alt+Delete screen",
        ["alt+tab"] = "switching windows",
        ["ctrl+alt+tab"] = "switching windows",
        ["alt+escape"] = "cycling through windows",
        ["alt+f4"] = "closing the window",
        ["alt+space"] = "the window menu",
        ["shift+f10"] = "the context menu",
        ["ctrl+escape"] = "the Start menu",
        ["ctrl+shift+escape"] = "Task Manager",
        ["win+space"] = "switching the keyboard layout",
        ["win+tab"] = "Task View",
        ["win+d"] = "showing the desktop",
        ["win+e"] = "File Explorer",
        ["win+r"] = "the Run box",
        ["win+i"] = "Settings",
        ["win+s"] = "search",
        ["win+a"] = "Quick Settings",
        ["win+n"] = "notifications",
        ["win+v"] = "clipboard history",
        ["win+h"] = "voice typing",
        ["win+period"] = "the emoji panel",
        ["shift+win+s"] = "a screenshot of a selection",
        ["win+left"] = "snapping a window to the left",
        ["win+right"] = "snapping a window to the right",
        ["win+up"] = "maximizing a window",
        ["win+down"] = "minimizing a window",
    };

    /// <summary>Shortcuts nearly every app gives the same meaning.</summary>
    public static IReadOnlyDictionary<string, string> AppShortcuts { get; } = new Dictionary<string, string>(StringComparer.Ordinal)
    {
        ["ctrl+c"] = "Copy",
        ["ctrl+v"] = "Paste",
        ["ctrl+x"] = "Cut",
        ["ctrl+z"] = "Undo",
        ["ctrl+y"] = "Redo",
        ["ctrl+a"] = "Select All",
        ["ctrl+s"] = "Save",
        ["ctrl+w"] = "Close",
        ["ctrl+n"] = "New",
        ["ctrl+o"] = "Open",
        ["ctrl+p"] = "Print",
        ["ctrl+f"] = "Find",
        ["ctrl+t"] = "New Tab",
    };
}
