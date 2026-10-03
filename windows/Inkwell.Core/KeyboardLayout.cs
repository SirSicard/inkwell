// The user's keyboard layout, for showing a key as it is labelled (KeyNotation.Describe): what the
// key at a virtual key types with no modifier in the current layout. Windows' MapVirtualKey; a
// dead key reads as its character too.
using System.Runtime.InteropServices;

namespace Inkwell.Core;

public static partial class KeyboardLayout
{
    private const uint MapVkToChar = 2;

    /// <summary>The character <paramref name="vk"/> types with no modifier, or null.</summary>
    public static string? Character(uint vk)
    {
        // The low word is the character; the high bit marks a dead key.
        var mapped = MapVirtualKeyW(vk, MapVkToChar) & 0xFFFF;
        return mapped == 0 ? null : ((char)mapped).ToString();
    }

    [LibraryImport("user32.dll")]
    private static partial uint MapVirtualKeyW(uint code, uint mapType);
}
