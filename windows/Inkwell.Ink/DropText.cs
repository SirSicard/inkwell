// The Drop's two lines, for the five states: the Mac's DropText for a held state
// (mac/Sources/Inkwell/ShellInk.swift). A dictation's lines come from the shell (the app's
// ShellInk, from Inkwell.Core's DropModel): while a key is held the second line is its live
// words, one line with the head cut and the newest words wet. The meeting's app and the consent
// offer's buttons come with meetings end to end.
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
/// <param name="LiveWords">
/// The detail is the live words of a take being held: its end matters (the head is cut, not the
/// tail), and its newest words are still wet (italic, muted).
/// </param>
public sealed record DropText(string Title, string Detail, DropTone Tone = DropTone.Plain, bool LiveWords = false)
{
    /// <summary>
    /// The Drop window's title, which any process can read (GetWindowText): "Inkwell: " and the
    /// title only. The detail can hold the user's live words, so it never goes into it.
    /// </summary>
    public string WindowTitle => $"Inkwell: {Title}";

    /// <summary>
    /// What a screen reader reads for the Drop, as on the Mac (Drop.swift's accessibility label):
    /// the title and the detail, live words included. It is not private to screen readers: while
    /// the Drop shows, any UI Automation or MSAA client running as the same user can read it, as
    /// any accessibility client can read the Mac's label. What stays word-free is the window's
    /// title and the logs.
    /// </summary>
    public string AccessibleName => $"Inkwell: {Title}, {Detail}";

    /// <summary>How many of the newest live words are shown wet, as on the canvas.</summary>
    public const int WetWords = 2;

    /// <summary>Where the wet words of <paramref name="detail"/> start: the last <see cref="WetWords"/> words (0 for fewer).</summary>
    public static int WetStart(string detail)
    {
        ArgumentNullException.ThrowIfNull(detail);
        var index = detail.Length;
        var words = 0;
        var inWord = false;
        for (var i = detail.Length - 1; i >= 0; i--)
        {
            var space = char.IsWhiteSpace(detail[i]);
            if (!space && !inWord)
            {
                words++;
                inWord = true;
            }
            else if (space && inWord)
            {
                inWord = false;
                if (words == WetWords)
                {
                    return index;
                }
            }
            if (!space)
            {
                index = i;
            }
        }
        return 0;
    }

    /// <summary>
    /// Live words cut from the head to fit one line of <paramref name="available"/> width, as
    /// <paramref name="measure"/> gives a string's width: whole words dropped from the start, an
    /// ellipsis in their place, so the newest words always show. A last word too long on its own
    /// is left for the layout to cut.
    /// </summary>
    public static string HeadCut(string words, Func<string, double> measure, double available)
    {
        ArgumentNullException.ThrowIfNull(words);
        ArgumentNullException.ThrowIfNull(measure);
        var text = words.Trim();
        if (measure(text) <= available)
        {
            return text;
        }
        // Where each word after the first starts; suffixes from later starts are shorter, so the
        // first start whose suffix fits is found by bisection.
        var starts = new List<int>();
        for (var i = 1; i < text.Length; i++)
        {
            if (!char.IsWhiteSpace(text[i]) && char.IsWhiteSpace(text[i - 1]))
            {
                starts.Add(i);
            }
        }
        if (starts.Count == 0)
        {
            return text;
        }
        string Cut(int k) => "\u2026" + text[starts[k]..];
        int lo = 0, hi = starts.Count - 1;
        while (lo < hi)
        {
            var mid = (lo + hi) / 2;
            if (measure(Cut(mid)) <= available)
            {
                hi = mid;
            }
            else
            {
                lo = mid + 1;
            }
        }
        return Cut(lo);
    }

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
