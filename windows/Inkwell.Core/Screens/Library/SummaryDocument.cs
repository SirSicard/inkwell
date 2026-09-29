// A summary as the screen shows it, as the Mac's SummaryDocument. The core stores summaries as
// markdown (a headline, the body in short sections, then Decisions, Actions and Open questions as
// lists); the earlier app printed that raw, `##` and `**` and all. Here it becomes blocks of runs
// of words, each run flagged bold, italic, code or struck through, and no markdown token survives
// into what is drawn. The view maps each block to a Paragraph of a RichTextBlock and each run to
// a Run with its weight, style and font; it never parses anything itself.
//
// Blocks are read line by line (headings, bullet and numbered lists, quotes, rules, code fences,
// paragraphs); inline markup (bold, italic, code, strikethrough, links) is read by a small parser
// here (the Mac uses Foundation's). Whatever it cannot pair is stripped of its markers instead:
// the reader sees the words, never the syntax.
//
// Links are words only. A summary is written by a language model from what the far end said, so
// a link in it is untrusted: its text stays, its destination is dropped (no run carries a link),
// and the views that show a summary refuse to open one anyway (SummaryLinks).
using System.Text;
using System.Text.RegularExpressions;

namespace Inkwell.Core.Screens;

/// <summary>Words drawn in one style.</summary>
public sealed record SummaryRun(string Text, bool Bold = false, bool Italic = false, bool Code = false, bool Strikethrough = false)
{
    internal bool SameStyle(SummaryRun other) =>
        Bold == other.Bold && Italic == other.Italic && Code == other.Code && Strikethrough == other.Strikethrough;
}

/// <summary>A line of styled words: its runs, in order.</summary>
public sealed record SummaryText(IReadOnlyList<SummaryRun> Runs)
{
    /// <summary>The words as drawn.</summary>
    public string Plain => string.Concat(Runs.Select(r => r.Text));

    public bool Equals(SummaryText? other) => other is not null && Runs.SequenceEqual(other.Runs);

    public override int GetHashCode() => Plain.GetHashCode(StringComparison.Ordinal);

    public override string ToString() => Plain;
}

/// <summary>One block of a rendered summary.</summary>
public abstract record SummaryBlock(SummaryText Text)
{
    /// <summary>A section heading (<c>##</c>), level 1 to 6.</summary>
    public sealed record Heading(int Level, SummaryText Text) : SummaryBlock(Text);

    /// <summary>A paragraph.</summary>
    public sealed record Paragraph(SummaryText Text) : SummaryBlock(Text);

    /// <summary>A list item; <paramref name="Marker"/> is <c>•</c> or the item's number (<c>1.</c>).</summary>
    public sealed record Item(string Marker, SummaryText Text) : SummaryBlock(Text);
}

/// <summary>A rendered summary: its headline (the first paragraph, which is also the record's title) and the blocks after it.</summary>
public sealed partial class SummaryDocument : IEquatable<SummaryDocument>
{
    public SummaryText? Headline { get; }

    public IReadOnlyList<SummaryBlock> Blocks { get; }

    /// <summary>Renders <paramref name="markdown"/>.</summary>
    public SummaryDocument(string markdown)
    {
        ArgumentNullException.ThrowIfNull(markdown);
        var blocks = Parse(markdown);
        if (blocks.Count > 0 && blocks[0] is SummaryBlock.Paragraph first)
        {
            Headline = first.Text;
            blocks.RemoveAt(0);
        }
        Blocks = blocks;
    }

    /// <summary>The whole summary as plain text, as drawn: what Copy puts on the clipboard, and what the snapshot test reads for markdown tokens.</summary>
    public string PlainText
    {
        get
        {
            var lines = new List<string>();
            if (Headline is not null)
            {
                lines.Add(Headline.Plain);
            }
            foreach (var block in Blocks)
            {
                switch (block)
                {
                    case SummaryBlock.Heading or SummaryBlock.Paragraph:
                        lines.Add("");
                        lines.Add(block.Text.Plain);
                        break;
                    case SummaryBlock.Item item:
                        lines.Add($"{item.Marker} {item.Text.Plain}");
                        break;
                }
            }
            return string.Join("\n", lines).Trim();
        }
    }

    /// <summary>The first paragraph after the headline, for the Today card: a sentence or two, no lists.</summary>
    public SummaryText? Lede => Blocks.OfType<SummaryBlock.Paragraph>().FirstOrDefault()?.Text;

    public bool Equals(SummaryDocument? other) =>
        other is not null && Equals(Headline, other.Headline) && Blocks.SequenceEqual(other.Blocks);

    public override bool Equals(object? obj) => Equals(obj as SummaryDocument);

    public override int GetHashCode() => HashCode.Combine(Headline, Blocks.Count);

    // Blocks

    private static List<SummaryBlock> Parse(string markdown)
    {
        var blocks = new List<SummaryBlock>();
        var paragraph = new List<string>();
        var inFence = false;
        void Flush()
        {
            var text = string.Join(" ", paragraph).Trim(' ', '\t');
            if (text.Length > 0)
            {
                blocks.Add(new SummaryBlock.Paragraph(Inline(text)));
            }
            paragraph.Clear();
        }
        foreach (var raw in markdown.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n'))
        {
            var line = raw.Trim(' ', '\t');
            if (line.StartsWith("```", StringComparison.Ordinal) || line.StartsWith("~~~", StringComparison.Ordinal))
            {
                Flush();
                inFence = !inFence;
                continue;
            }
            if (inFence)
            {
                // Code is shown as its text, one paragraph per line.
                if (line.Length > 0)
                {
                    blocks.Add(new SummaryBlock.Paragraph(new SummaryText([new SummaryRun(line)])));
                }
                continue;
            }
            if (line.Length == 0 || IsRule(line))
            {
                Flush();
                continue;
            }
            if (HeadingOf(line) is var (level, heading))
            {
                Flush();
                blocks.Add(new SummaryBlock.Heading(level, Inline(heading)));
                continue;
            }
            if (Bullet(line) is string bullet)
            {
                Flush();
                blocks.Add(new SummaryBlock.Item("•", Inline(bullet)));
                continue;
            }
            if (Numbered(line) is var (number, numbered))
            {
                Flush();
                blocks.Add(new SummaryBlock.Item($"{number}.", Inline(numbered)));
                continue;
            }
            if (line.StartsWith('>'))
            {
                paragraph.Add(line.TrimStart('>', ' '));
                continue;
            }
            paragraph.Add(line);
        }
        Flush();
        return blocks;
    }

    private static bool IsRule(string line)
    {
        var chars = line.Where(c => c != ' ').ToList();
        return chars.Count >= 3 && chars.Distinct().Count() == 1 && chars[0] is '-' or '*' or '_';
    }

    private static (int, string)? HeadingOf(string line)
    {
        var hashes = line.TakeWhile(c => c == '#').Count();
        if (hashes is < 1 or > 6 || line.Length <= hashes || line[hashes] != ' ')
        {
            return null;
        }
        var text = line[hashes..].Trim(' ', '\t');
        // A closing run of #s is part of the syntax too.
        return (hashes, ClosingHashes().Replace(text, ""));
    }

    private static string? Bullet(string line)
    {
        foreach (var marker in (string[])["- ", "* ", "+ ", "• "])
        {
            if (!line.StartsWith(marker, StringComparison.Ordinal))
            {
                continue;
            }
            var text = line[marker.Length..];
            // A task list's box is syntax as well.
            foreach (var box in (string[])["[ ] ", "[x] ", "[X] "])
            {
                if (text.StartsWith(box, StringComparison.Ordinal))
                {
                    text = text[box.Length..];
                }
            }
            return text;
        }
        return null;
    }

    private static (int, string)? Numbered(string line)
    {
        var digits = line.TakeWhile(char.IsAsciiDigit).Count();
        if (digits is < 1 or > 3)
        {
            return null;
        }
        var rest = line[digits..];
        if (!rest.StartsWith(". ", StringComparison.Ordinal) && !rest.StartsWith(") ", StringComparison.Ordinal))
        {
            return null;
        }
        return (int.Parse(line.AsSpan(0, digits), System.Globalization.CultureInfo.InvariantCulture), rest[2..]);
    }

    // Inline

    /// <summary>Bold, italic, code, links and strikethrough become flags on runs; their markers go.</summary>
    public static SummaryText Inline(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        var runs = Merge(InlineParser.Parse(text, new SummaryRun("")));
        var parsed = new SummaryText(runs);
        if (!ContainsMarkup(parsed.Plain))
        {
            return parsed;
        }
        return new SummaryText([new SummaryRun(Stripped(text))]);
    }

    /// <summary>Whether <paramref name="text"/> still holds inline markup: paired emphasis markers, code ticks, a link.</summary>
    public static bool ContainsMarkup(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        return text.Contains("**", StringComparison.Ordinal) || text.Contains("__", StringComparison.Ordinal)
            || text.Contains('`', StringComparison.Ordinal) || text.Contains("~~", StringComparison.Ordinal)
            || LinkTail().IsMatch(text);
    }

    /// <summary><paramref name="text"/> with inline markers removed, keeping the words (and a link's text).</summary>
    public static string Stripped(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        var output = WholeLink().Replace(text, "$1");
        foreach (var marker in (string[])["**", "__", "~~", "`"])
        {
            output = output.Replace(marker, "", StringComparison.Ordinal);
        }
        // Single * or _ around a word (italic), not an apostrophe or a snake_case name.
        return SingleEmphasis().Replace(output, "$1");
    }

    private static List<SummaryRun> Merge(List<SummaryRun> runs)
    {
        var merged = new List<SummaryRun>();
        foreach (var run in runs.Where(r => r.Text.Length > 0))
        {
            if (merged.Count > 0 && merged[^1].SameStyle(run))
            {
                merged[^1] = merged[^1] with { Text = merged[^1].Text + run.Text };
            }
            else
            {
                merged.Add(run);
            }
        }
        return merged;
    }

    [GeneratedRegex(@"\s+#+$")]
    private static partial Regex ClosingHashes();

    [GeneratedRegex(@"\]\([^)]*\)")]
    private static partial Regex LinkTail();

    [GeneratedRegex(@"!?\[([^\]]*)\]\([^)]*\)")]
    internal static partial Regex WholeLink();

    [GeneratedRegex(@"(?<![\w*])[*_](\S(?:.*?\S)?)[*_](?![\w*])")]
    private static partial Regex SingleEmphasis();

    [GeneratedRegex(@"\G!?\[([^\]]*)\]\([^)]*\)")]
    internal static partial Regex LinkAt();

    [GeneratedRegex(@"\G<([A-Za-z][A-Za-z0-9+.\-]{1,31}:[^<>\s]*|[^<>\s@]+@[^<>\s]+)>")]
    internal static partial Regex AutolinkAt();
}

/// <summary>
/// Reads inline markdown into runs: code spans, links (their words), autolinks (their address as
/// words), <c>**</c>/<c>__</c> bold, <c>*</c>/<c>_</c> italic, <c>~~</c> strikethrough, and
/// backslash escapes. A marker it cannot pair stays as it is written (SummaryDocument.Inline then
/// strips the line).
/// </summary>
internal static class InlineParser
{
    public static List<SummaryRun> Parse(string s, SummaryRun style)
    {
        var runs = new List<SummaryRun>();
        var plain = new StringBuilder();
        void Flush()
        {
            if (plain.Length > 0)
            {
                runs.Add(style with { Text = plain.ToString() });
                plain.Clear();
            }
        }
        var i = 0;
        while (i < s.Length)
        {
            var c = s[i];
            if (c == '\\' && i + 1 < s.Length && IsEscapable(s[i + 1]))
            {
                plain.Append(s[i + 1]);
                i += 2;
                continue;
            }
            if (c == '`')
            {
                var ticks = RunLength(s, i, '`');
                var close = s.IndexOf(new string('`', ticks), i + ticks, StringComparison.Ordinal);
                if (close > i + ticks)
                {
                    Flush();
                    var code = s[(i + ticks)..close];
                    if (code.Length > 2 && code[0] == ' ' && code[^1] == ' ')
                    {
                        code = code[1..^1];
                    }
                    runs.Add(style with { Text = code, Code = true });
                    i = close + ticks;
                    continue;
                }
                plain.Append(s, i, ticks);
                i += ticks;
                continue;
            }
            if (c == '[' || (c == '!' && i + 1 < s.Length && s[i + 1] == '['))
            {
                var link = SummaryDocument.LinkAt().Match(s, i);
                if (link.Success)
                {
                    Flush();
                    // The words of a link stay; where it goes does not.
                    runs.AddRange(Parse(link.Groups[1].Value, style));
                    i += link.Length;
                    continue;
                }
            }
            if (c == '<')
            {
                var auto = SummaryDocument.AutolinkAt().Match(s, i);
                if (auto.Success)
                {
                    plain.Append(auto.Groups[1].Value);
                    i += auto.Length;
                    continue;
                }
            }
            if (c is '*' or '_' or '~')
            {
                var n = RunLength(s, i, c);
                if (Emphasis(s, i, c, n, style) is var (inner, styled, end))
                {
                    Flush();
                    runs.AddRange(Parse(inner, styled));
                    i = end;
                    continue;
                }
                plain.Append(s, i, n);
                i += n;
                continue;
            }
            plain.Append(c);
            i++;
        }
        Flush();
        return runs;
    }

    /// <summary>
    /// The span a run of <paramref name="n"/> markers at <paramref name="i"/> opens, if a matching
    /// run closes it: its inner text, its style, and where it ends.
    /// </summary>
    private static (string, SummaryRun, int)? Emphasis(string s, int i, char c, int n, SummaryRun style)
    {
        var width = c == '~' ? (n >= 2 ? 2 : 0) : Math.Min(n, 3);
        if (width == 0 || n != width)
        {
            return null;
        }
        var open = i + width;
        // An opener is followed by a word, and an underscore opener is not inside one (snake_case).
        if (open >= s.Length || char.IsWhiteSpace(s[open]) || (c == '_' && i > 0 && char.IsLetterOrDigit(s[i - 1])))
        {
            return null;
        }
        var j = open;
        while (j < s.Length)
        {
            if (s[j] == '`')
            {
                // Markers inside a code span are its words.
                var ticks = RunLength(s, j, '`');
                var close = s.IndexOf(new string('`', ticks), j + ticks, StringComparison.Ordinal);
                j = close > 0 ? close + ticks : j + ticks;
                continue;
            }
            if (s[j] != c)
            {
                j++;
                continue;
            }
            var m = RunLength(s, j, c);
            var closes = m == width && j > open && !char.IsWhiteSpace(s[j - 1])
                && (c != '_' || j + m >= s.Length || !char.IsLetterOrDigit(s[j + m]));
            if (closes)
            {
                var styled = c switch
                {
                    '~' => style with { Strikethrough = true },
                    _ when width == 1 => style with { Italic = true },
                    _ when width == 2 => style with { Bold = true },
                    _ => style with { Bold = true, Italic = true },
                };
                return (s[open..j], styled, j + m);
            }
            j += m;
        }
        return null;
    }

    /// <summary>ASCII punctuation, which a backslash makes literal.</summary>
    private static bool IsEscapable(char c) => c < 128 && (char.IsPunctuation(c) || char.IsSymbol(c));

    private static int RunLength(string s, int i, char c)
    {
        var n = 0;
        while (i + n < s.Length && s[i + n] == c)
        {
            n++;
        }
        return n;
    }
}

/// <summary>
/// What the views showing a summary do with a link: refuse it. The summary's runs carry none, so
/// this is the second line: nothing a summary holds opens a URL.
/// </summary>
public static class SummaryLinks
{
    public enum Decision
    {
        Refused,
    }

    public static Decision Decide(Uri url) => Decision.Refused;
}
