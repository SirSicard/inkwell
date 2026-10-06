// The Live screen's ledger and header, as the Mac's LiveLine and LiveMeetingView: what is being
// said, read from the CoreStore's LiveMeeting. Finals are dry (settled, in the record) and partials
// are wet (the engine's current guess, grey and italic, replaced by the next one). Partials are
// shown and never stored: they live only in the store's live meeting until the next partial or
// final replaces them (architecture rule 4).
using System.Globalization;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>One line of the ledger.</summary>
/// <param name="AtMs">Where in the meeting it started, ms; null for a partial.</param>
/// <param name="Wet">Still settling: a partial.</param>
public sealed record LiveLine(string Id, Channel Channel, long? AtMs, string Text, bool Wet)
{
    /// <summary>
    /// Everything the core has for the live meeting: the finals in the order they were said (each
    /// side settles at its own pace, so they arrive out of order), then each side's partial.
    /// </summary>
    public static IReadOnlyList<LiveLine> Ledger(LiveMeeting meeting)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        // OrderBy is stable: lines said at the same moment keep their arrival order.
        var lines = meeting.Finals
            .Select((f, i) => new LiveLine($"final-{i.ToString(CultureInfo.InvariantCulture)}", f.Channel, f.StartMs, f.Text, false))
            .OrderBy(l => l.AtMs ?? 0)
            .ToList();
        foreach (var channel in (Channel[])[Channel.Mic, Channel.Far])
        {
            if (meeting.Partials.TryGetValue(channel, out var partial) && !string.IsNullOrWhiteSpace(partial))
            {
                lines.Add(new LiveLine($"partial-{Wire.Name(channel)}", channel, null, partial, true));
            }
        }
        return lines;
    }

    /// <summary>The user's own line (the mic channel).</summary>
    public bool Mine => Channel == Channel.Mic;

    /// <summary>"You" or "Them".</summary>
    public string Speaker => Mine ? "You" : "Them";

    /// <summary>The row's clock ("12:18"), empty for a partial.</summary>
    public string Clock => AtMs is long at ? LiveClock.Of(at) : "";

    /// <summary>What a screen reader says for the row.</summary>
    public string AccessibilityText => Wet
        ? $"{Speaker}, still settling: {Text}"
        : $"{Speaker}{(AtMs is long at ? $" at {LiveClock.Of(at)}" : "")}: {Text}";
}

public static class LiveClock
{
    /// <summary>"12:41": minutes and seconds into the meeting (hours when past one).</summary>
    public static string Of(long ms)
    {
        var seconds = Math.Max(0, ms / 1_000);
        var (h, m, s) = (seconds / 3_600, seconds % 3_600 / 60, seconds % 60);
        return h > 0
            ? string.Create(CultureInfo.InvariantCulture, $"{h}:{m:00}:{s:00}")
            : string.Create(CultureInfo.InvariantCulture, $"{m}:{s:00}");
    }
}

/// <summary>What the other side is, in words, and whether it is said as a warning.</summary>
public sealed record FarEndLine(string Text, bool Alert);

/// <summary>The Live screen's header and ledger texts for a live meeting (the Mac's LiveMeetingView).</summary>
public static class LiveHeader
{
    /// <summary>
    /// The meeting's name: its title, else its app's call, else "Live meeting" (the core names an
    /// untitled meeting later, from the summary's headline).
    /// </summary>
    public static string Title(LiveMeeting meeting)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        return meeting.Title ?? (meeting.AppName is string app ? $"{app} call" : "Live meeting");
    }

    /// <summary>The app, beside the status, when the title does not already name it.</summary>
    public static string? AppLine(LiveMeeting meeting)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        return meeting.Title is not null ? meeting.AppName : null;
    }

    /// <summary>
    /// What the other side is, when it is more than the call's app: everything this PC plays, for
    /// Record now, or because the call's app could not be heard alone (said as a warning: other
    /// apps' sound is in the recording).
    /// </summary>
    public static FarEndLine? FarEnd(LiveMeeting meeting)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        if (meeting.FarEndFallback)
        {
            var app = meeting.AppName ?? "the call";
            return new FarEndLine($"Inkwell couldn't hear {app} alone, so it is recording everything this PC plays.", true);
        }
        if (meeting.FarEnd == Events.FarEnd.Everything)
        {
            return new FarEndLine("Recording everything this PC plays, as well as your microphone.", false);
        }
        return null;
    }

    /// <summary>
    /// Which mic, when the reason is worth saying ("why is it using the laptop mic?"): a mic that went
    /// and the one in its place, one standing in for a chosen mic that isn't connected, or
    /// Automatic's reason.
    /// </summary>
    public static string? MicLine(LiveMeeting meeting)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        if (meeting.MicName is not string name)
        {
            return null;
        }
        if (meeting.MicSwitch is { } change)
        {
            return change.From is { } from ? $"{name}, since {from} went" : $"{name}, since your mic went";
        }
        return meeting.MicReason switch
        {
            MicReason.ChosenMissing => $"{name}, until your chosen mic is back",
            MicReason.BuiltInForBluetoothOutput => $"{name}, because your headphones are Bluetooth",
            MicReason.HeadsetMicSetting => $"{name}, the headset's own mic",
            // An LE Audio headset keeps full quality on its own mic, so Windows records it.
            MicReason.LeAudioHeadset => $"{name}, the headset's own mic",
            _ => null,
        };
    }

    /// <summary>A side that stopped or delivers only silence, in words (shown in the alert colour).</summary>
    public static IReadOnlyList<string> SideWarnings(LiveMeeting meeting)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        var far = meeting.Sides.GetValueOrDefault(Channel.Far, SideState.Ok);
        var mic = meeting.Sides.GetValueOrDefault(Channel.Mic, SideState.Ok);
        var warnings = new List<string>();
        if (far == SideState.Zeros)
        {
            warnings.Add("The others' audio is silent");
        }
        if (far == SideState.Stopped)
        {
            warnings.Add("The others' audio stopped");
        }
        if (mic == SideState.Zeros)
        {
            warnings.Add("Your microphone is silent");
        }
        if (mic == SideState.Stopped)
        {
            warnings.Add("Your microphone stopped");
        }
        return warnings;
    }

    /// <summary>The oldest lines were let go of in memory: the ledger says "Earlier lines are in the record."</summary>
    public static bool EarlierInRecord(LiveMeeting meeting)
    {
        ArgumentNullException.ThrowIfNull(meeting);
        return meeting.Ledger.Dropped > 0;
    }
}

/// <summary>
/// The Live screen's minimum widths, as the Mac's LiveMeetingView sets them. The window follows its
/// content's minimum size, so nothing on this screen may ask for more as the meeting or the notes
/// grow: the notes editor and the ledger scroll inside, and their minimum widths are fixed here
/// for the view to bind (a text squeezed to no width would be as tall as its letters).
/// </summary>
public static class LiveLayout
{
    /// <summary>The window's minimum content size (the Mac's is 720 by 460).</summary>
    public const double WindowMinWidth = 720;
    public const double WindowMinHeight = 460;

    public const double NotesMinWidth = 200;
    public const double LedgerMinWidth = 240;
    /// <summary>Between each column and the rule that divides them.</summary>
    public const double ColumnGutter = 26;
    public const double RuleWidth = 1;
    /// <summary>The screen's padding, each side.</summary>
    public const double SidePadding = 36;

    /// <summary>The least width the Live screen takes, whatever it shows.</summary>
    public static double MinimumWidth =>
        SidePadding + NotesMinWidth + ColumnGutter + RuleWidth + ColumnGutter + LedgerMinWidth + SidePadding;
}
