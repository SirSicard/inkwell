// A record as the Record screen reads it (the Mac's RecordDocumentTests, and CitedDecisionTests
// from MeetingsTests): the ledger, the notes-first merge, what is owed and where it was said, and
// each summary item's cited line.
using Inkwell.Core.Events;
using Inkwell.Core.Screens;
using Xunit;
using static Inkwell.Core.Tests.Screens.LibraryFixtures;

namespace Inkwell.Core.Tests.Screens;

public class RecordDocumentTests
{
    [Fact]
    public void TheLedgerNamesEachSpeakerByStreamAndName()
    {
        var doc = Document();
        Assert.Equal(["Alex", "You", "You", "Speaker 1", "Speaker 2", "Them"], doc.Ledger.Select(l => l.Speaker.Label));
        Assert.Equal(["Alex", "Speaker 1", "Speaker 2", "Them"], doc.People);
        Assert.True(doc.IsFinal);
        Assert.Equal(30_003, doc.DurationMs); // the audio's end
        Assert.Equal("Can you share the budget?", doc.LineAtPlayhead(11_000)?.Text);
        Assert.Null(doc.LineAtPlayhead(-1));
    }

    [Fact]
    public void NotesComeFirstEachFilledInWithWhatWasSaidAndPromisedThen()
    {
        var merged = Document().Merged.Select(entry => entry.Kind switch
        {
            MergedKind.Note => $"note {entry.Text} @{entry.AtMs}",
            MergedKind.Said => $"said {entry.Speaker!.Label} @{entry.AtMs}",
            _ => $"owed {entry.Text} @{entry.AtMs}",
        });
        Assert.Equal(
        [
            "note Launch date @2000",
            "said Alex @0", "said You @486",
            "owed Send the revised plan @2822", "owed Share the budget @10000",
            "note Beta: small group @15000",
            "said Speaker 1 @10000", "said Speaker 2 @18278",
        ], merged);
    }

    [Fact]
    public void OwedListsWhatStandsWithTheLineItWasSaidIn()
    {
        var doc = Document();
        Assert.Equal(["c2", "c3"], doc.Owed.Select(o => o.Id)); // the merged duplicate is folded away
        Assert.Equal("I'll send the revised plan by Friday.", doc.Owed[0].CitedLine?.Text);
        Assert.Equal(Speaker.Them("Speaker 1"), doc.Owed[1].CitedLine?.Speaker);
        Assert.True(doc.Owed[1].Done);
        Assert.Equal("Launch moves to the 14th", doc.Summary?.Headline?.Plain);
        Assert.Equal(2, doc.Chunks.Count);
    }

    /// <summary>Windows addition: the words the Record screen composes from the document (the Mac composes them in RecordScreen's views).</summary>
    [Fact]
    public void TheRecordScreensWordsComeFromTheDocument()
    {
        var doc = Document();
        Assert.Equal("Owed · 1", doc.TabTitle(RecordTab.Owed));
        Assert.Equal("Notes and transcript", doc.TabTitle(RecordTab.Notes));
        Assert.Equal(["You", "Alex", "Speaker 1", "Speaker 2", "Them"], doc.HeaderPeople);
        Assert.Equal("Your notes, filled in", doc.NotesHeading);
        Assert.Null(doc.NoNotesText);
        Assert.StartsWith("00:00 Alex: The design review needs another week.\n00:00 You: Let's settle", doc.TranscriptText, StringComparison.Ordinal);
        Assert.Equal("Launch moves to the 14th\n\nLaunch moves to the 14th\n\nThe review needs another week.", doc.ShareText);
        Assert.Equal("Friday", doc.Owed[0].MetaLine);
        Assert.Equal("Mark done: Send the revised plan", doc.Owed[0].ToggleLabel);
        Assert.Equal(["c2", "c3"], doc.CitedOwed.Select(o => o.Id));
        Assert.Equal("Play from 12:41", RecordDocument.ChipLabel(761_000));
        Assert.Equal("You: “I'll send the revised plan by Friday.”", RecordDocument.QuoteOf(doc.Owed[0].CitedLine!));
        Assert.Equal("Summaries are off.", RecordDocument.NoSummaryText("Summaries are off."));
        Assert.Equal("A summary is written after a meeting ends, when a language model is set up.", RecordDocument.NoSummaryText(null));
    }
}

public class CitedDecisionTests
{
    /// <summary>Carried into S2.8 from S2.5: a decision shows the line it cites, from the item's span.</summary>
    [Fact]
    public void ASummarysDecisionsCarryTheirCitedLines()
    {
        var answer = Ev.Of<LibraryRecord>("""
            {"type":"library.record","ref":"x","record":{"record":"r1","kind":"meeting","started_at_unix_ms":0,"revision":2,"has_audio":false},
             "segments":[{"channel":"far","start_ms":0,"end_ms":2000,"text":"Let us start on the fourteenth."},
                         {"channel":"mic","start_ms":3000,"end_ms":5000,"text":"Agreed, the fourteenth it is."}],
             "notes":[],"commitments":[],"speakers":[],
             "summary":{"text":"Start date set.","model":"m","created_at_unix_ms":1,
                        "items":[{"kind":"decision","text":"Start on the fourteenth","span":{"channel":"mic","start_ms":3000,"end_ms":5000}},
                                 {"kind":"action","text":"Book the room","span":{"channel":"far","start_ms":0,"end_ms":2000}}]}}
            """);
        var document = new RecordDocument(answer);
        Assert.Equal([SummaryItemKind.Decision, SummaryItemKind.Action], document.SummaryItems.Select(i => i.Kind));
        Assert.Equal("Agreed, the fourteenth it is.", document.SummaryItems[0].CitedLine?.Text);
        Assert.True(document.SummaryItems[0].CitedLine?.Speaker.IsYou);
        Assert.Equal("Let us start on the fourteenth.", document.SummaryItems[1].CitedLine?.Text);
        Assert.Equal(["Start on the fourteenth"], document.Decisions.Select(d => d.Text));
        Assert.Equal("You: “Agreed, the fourteenth it is.”", document.SummaryItems[0].Quote);
    }
}
