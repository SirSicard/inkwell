// The Drop's two lines, for the five states: the Mac's DropText for a held state
// (mac/Sources/Inkwell/ShellInk.swift). The live words, the meeting's app and the consent offer's
// buttons come with dictation and meetings end to end.
namespace Inkwell.Ink;

/// <summary>How the Drop colours its title and border.</summary>
public enum DropTone
{
    /// <summary>Muted title, hairline border.</summary>
    Plain,
    /// <summary>A recording: the title in seal red, as a recording light.</summary>
    Recording,
    /// <summary>Something needs the user: the title and the border in seal red.</summary>
    Alert,
}

/// <summary>What the Drop says beside the ink.</summary>
public sealed record DropText(string Title, string Detail, DropTone Tone = DropTone.Plain)
{
    /// <summary>The lines for a state.</summary>
    public static DropText For(InkState state) => state switch
    {
        InkState.Dictating => new("Dictating", "Listening"),
        InkState.Meeting => new("● REC", "Recording this meeting", DropTone.Recording),
        InkState.Blotting => new("Blotting", "The final pass"),
        InkState.Problem => new("Far end silent", "Nothing is arriving from the call", DropTone.Alert),
        _ => new("", ""),
    };
}

/// <summary>The design tokens the Drop paints with (the Mac's Palette).</summary>
internal static class Palette
{
    public static readonly (float R, float G, float B) Paper = Rgb(0xF2EEE6);
    public static readonly (float R, float G, float B) Ink = Rgb(0x16181F);
    public static readonly (float R, float G, float B) Muted = Rgb(0x625E57);
    public static readonly (float R, float G, float B) Seal = Rgb(0xB23A26);

    private static (float, float, float) Rgb(int hex) =>
        (((hex >> 16) & 0xFF) / 255f, ((hex >> 8) & 0xFF) / 255f, (hex & 0xFF) / 255f);
}
