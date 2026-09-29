// Today's foot of the ink zone, as the Mac's RecordControls (ShellView.swift): whether Inkwell
// listens for calls, which key dictates, and Record now (the meetings model's command; its failure
// is that model's to show). The listening line follows the core's state (meeting.detection), never
// the setting. The Mac's line is a fixed "Hold fn to dictate"; Windows names the key the core
// bound, from dictation.ready ("Hold Right Ctrl to dictate"), and says nothing before it knows.
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>What the ink zone's foot says. UI thread only; fed every event through Apply.</summary>
public sealed class RecordControlsModel : ObservableModel
{
    /// <summary>Record now's accessible hint.</summary>
    public const string RecordNowHint = "Records the mic and everything this PC plays, until you stop it";

    /// <summary>The dictation key's token, as the core bound it (dictation.ready); null until it says.</summary>
    public string? DictationKey { get; private set; }

    /// <summary>"Hold Right Ctrl to dictate"; null until the core has bound a key.</summary>
    public string? DictateText => DictationKey is { } key ? $"Hold {KeyNames.Display(key)} to dictate" : null;

    public void Apply(InkEvent e)
    {
        if (e is DictationReady ready && ready.Key != DictationKey)
        {
            DictationKey = ready.Key;
            Changed();
        }
    }

    /// <summary>The core's detection state in words (a single space, holding the line's height, until the core says).</summary>
    public static string ListeningText(bool recording, bool? listening) =>
        recording ? "Recording"
        : listening switch
        {
            true => "Listening for meetings",
            false => "Not listening for meetings",
            null => " ",
        };
}

/// <summary>A hotkey token in Windows' words: right_control is "Right Ctrl", ctrl+shift+space is "Ctrl+Shift+Space", f13 is "F13" (the tokens and aliases ink-platform-win's hotkey binding accepts).</summary>
public static class KeyNames
{
    public static string Display(string token)
    {
        ArgumentNullException.ThrowIfNull(token);
        return string.Join('+', token.Split('+').Select(Part));
    }

    private static string Part(string part) => part.ToLowerInvariant() switch
    {
        "right_control" or "right_ctrl" => "Right Ctrl",
        "right_alt" or "right_option" or "right_opt" or "altgr" => "Right Alt",
        "right_shift" => "Right Shift",
        "right_win" or "right_super" => "Right Windows key",
        "ctrl" or "control" => "Ctrl",
        "alt" or "option" or "opt" => "Alt",
        "shift" => "Shift",
        "win" or "super" or "meta" => "Windows key",
        "return" or "enter" => "Enter",
        "escape" or "esc" => "Esc",
        "space" => "Space",
        "" => part,
        var other => char.ToUpperInvariant(other[0]) + other[1..],
    };
}
