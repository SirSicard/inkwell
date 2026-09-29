// The step's snapshot check (the Mac's SummaryRenderingTests and SummaryLinkTests): the rendered
// summary contains no markdown tokens, what was marked up is styled, not lost, and no run carries
// a link.
using System.Text.RegularExpressions;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.LibraryFixtures;

namespace Inkwell.Core.Tests.Screens;

public class SummaryRenderingTests
{
    /// <summary>Markdown syntax that must never reach the screen.</summary>
    internal static void AssertNoMarkdown(string text)
    {
        foreach (var token in (string[])["**", "__", "`", "~~", "](", "[x]", "[ ]"])
        {
            Assert.False(text.Contains(token, StringComparison.Ordinal), $"“{token}” in:\n{text}");
        }
        foreach (var line in text.Split('\n'))
        {
            var t = line.Trim(' ');
            Assert.False(t.StartsWith('#'), $"a heading marker in: {line}");
            Assert.False(t.StartsWith("- ", StringComparison.Ordinal) || t.StartsWith("* ", StringComparison.Ordinal) || t.StartsWith("+ ", StringComparison.Ordinal), $"a list marker in: {line}");
            Assert.False(t.StartsWith('>'), $"a quote marker in: {line}");
            Assert.False(t is "---" or "***", $"a rule in: {line}");
        }
        Assert.False(Regex.IsMatch(text, @"(?<!\w)\*\S[^*]*\*(?!\w)"), $"italic stars in:\n{text}");
    }

    [Fact]
    public void TheStoredSummaryRendersWithoutMarkdownTokens()
    {
        var doc = new SummaryDocument(SeededSummary);
        var snapshot = doc.PlainText;
        AssertNoMarkdown(snapshot);
        Assert.Equal("Launch moves to the 14th, after the design review", doc.Headline?.Plain);
        Assert.Contains("The design review needs another week, so the launch moves to the fourteenth.", snapshot, StringComparison.Ordinal);
        Assert.Contains("• Keep the beta group small, about forty people", snapshot, StringComparison.Ordinal);
        Assert.Contains("1. Revised plan first", snapshot, StringComparison.Ordinal);
        Assert.Contains("\nDecisions\n", snapshot, StringComparison.Ordinal); // a section heading is its words
        var headings = doc.Blocks.OfType<SummaryBlock.Heading>().Select(h => h.Text.Plain);
        Assert.Equal(["Launch date", "Beta", "Decisions", "Actions", "Open questions"], headings);
    }

    [Fact]
    public void EmphasisIsStyledNotDropped()
    {
        var doc = new SummaryDocument(SeededSummary);
        var body = doc.Blocks.OfType<SummaryBlock.Paragraph>().First().Text;
        Assert.Equal(["another week"], body.Runs.Where(r => r.Bold).Select(r => r.Text));
        Assert.Equal(["fourteenth"], body.Runs.Where(r => r.Italic).Select(r => r.Text));
    }

    [Fact]
    public void AnythingAModelMightWriteLosesItsSyntax()
    {
        const string messy = """
            # Weekly sync #
            Status: **green** and __steady__, see [the plan](https://example.com/plan) and `build 42`.
            > Quoted from the call: ~~never~~ always ship on Friday.

            ---
            * [ ] draft the notes
            + [x] book the room
            10) numbered with a paren
            ```
            code stays as its words
            ```
            A **broken bold at the end
            """;
        var snapshot = new SummaryDocument(messy).PlainText;
        AssertNoMarkdown(snapshot);
        foreach (var words in (string[])["Weekly sync", "green and steady", "the plan", "build 42", "always ship on Friday",
                     "draft the notes", "book the room", "10. numbered with a paren", "code stays as its words",
                     "A broken bold at the end"])
        {
            Assert.True(snapshot.Contains(words, StringComparison.Ordinal), $"“{words}” missing from:\n{snapshot}");
        }
        Assert.DoesNotContain("example.com", snapshot, StringComparison.Ordinal); // a link shows its text, not its address
    }

    /// <summary>Windows addition: the runs the view maps to a RichTextBlock carry code and strikethrough as flags, and an unpaired marker never leaves a styled fragment behind.</summary>
    [Fact]
    public void CodeAndStrikethroughAreFlagsOnTheirRuns()
    {
        var line = SummaryDocument.Inline("Ship ~~Friday~~ Monday with `build 42` and snake_case_names intact");
        Assert.Equal("Ship Friday Monday with build 42 and snake_case_names intact", line.Plain);
        Assert.Equal(["Friday"], line.Runs.Where(r => r.Strikethrough).Select(r => r.Text));
        Assert.Equal(["build 42"], line.Runs.Where(r => r.Code).Select(r => r.Text));
        Assert.DoesNotContain(line.Runs, r => r.Italic);
        var broken = SummaryDocument.Inline("A **broken bold and *half");
        Assert.Equal([new SummaryRun("A broken bold and *half")], broken.Runs);
    }
}

/// <summary>
/// Review fix (security): a summary is written by a language model from what the far end said, so
/// a link in it is untrusted. The rendered summary keeps a link's words and drops where it goes:
/// no run anywhere carries a link. (On Windows a run has no link field at all; the check is that
/// no destination survives as words either.)
/// </summary>
public class SummaryLinkTests
{
    private static IEnumerable<SummaryText> AllRuns(SummaryDocument doc)
    {
        if (doc.Headline is { } headline)
        {
            yield return headline;
        }
        foreach (var block in doc.Blocks)
        {
            yield return block.Text;
        }
    }

    [Fact]
    public void NoRenderedRunCarriesALinkAndTheWordsStay()
    {
        const string markdown = """
            Call [the vendor](https://evil.example/pay) today, see <https://evil.example/auto>.

            ## Actions
            - Read [**the brief**](http://evil.example/x "title") first
            - Mail [me](mailto:someone@example.com)
            """;
        var doc = new SummaryDocument(markdown);
        foreach (var text in AllRuns(doc))
        {
            Assert.DoesNotContain("evil.example/pay", text.Plain, StringComparison.Ordinal);
            Assert.DoesNotContain("evil.example/x", text.Plain, StringComparison.Ordinal);
            Assert.DoesNotContain("mailto:", text.Plain, StringComparison.Ordinal);
        }
        var plain = doc.PlainText;
        foreach (var words in (string[])["Call the vendor today", "Read the brief first", "Mail me"])
        {
            Assert.True(plain.Contains(words, StringComparison.Ordinal), $"“{words}” missing from:\n{plain}");
        }
        Assert.DoesNotContain("evil.example/pay", plain, StringComparison.Ordinal); // a hidden destination never shows as text either
        Assert.Equal(["the brief"], doc.Blocks.SelectMany(b => b.Text.Runs).Where(r => r.Bold).Select(r => r.Text));
        Assert.Null(doc.Lede);
    }

    [Fact]
    public void TheViewsThatShowASummaryRefuseToOpenLinks()
    {
        Assert.Equal(SummaryLinks.Decision.Refused, SummaryLinks.Decide(new Uri("https://evil.example")));
        Assert.Equal(SummaryLinks.Decision.Refused, SummaryLinks.Decide(new Uri("mailto:a@b.example")));
    }
}
