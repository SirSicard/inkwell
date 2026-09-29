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
    public string? DictateText => DictationKey is { } key ? $"Hold {DictationModel.Key(key)?.Name ?? DictationModel.Cap(key)} to dictate" : null;

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
