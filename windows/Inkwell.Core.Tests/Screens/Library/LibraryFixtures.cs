// What the Library tests share (the Mac LibraryTests' helpers): record rows as the core writes
// them, the seeded summary, a whole record's answer, and a search delay the test fires by hand.
using System.Globalization;
using System.Text.Json;
using Inkwell.Core.Events;
using Inkwell.Core.Screens;

namespace Inkwell.Core.Tests.Screens;

internal static class LibraryFixtures
{
    public static string Row(string id, string kind = "meeting", long start = 0, long? end = null, string? title = null)
    {
        var fields = new List<string>
        {
            $"\"record\":\"{id}\"", $"\"kind\":\"{kind}\"",
            string.Create(CultureInfo.InvariantCulture, $"\"started_at_unix_ms\":{start}"), "\"revision\":2", "\"has_audio\":false",
        };
        if (end is long e)
        {
            fields.Add(string.Create(CultureInfo.InvariantCulture, $"\"ended_at_unix_ms\":{e}"));
        }
        if (title is not null)
        {
            fields.Add($"\"title\":\"{title}\"");
        }
        return "{" + string.Join(",", fields) + "}";
    }

    public static IReadOnlyList<RecordRow> Rows(string answer) => Ev.Of<LibraryRecords>(answer).Records;

    /// <summary>A command's JSON fields.</summary>
    public static JsonElement Fields(CoreCommand command) => JsonDocument.Parse(command.Json).RootElement;

    /// <summary>The id a command carried.</summary>
    public static string RequestId(CoreCommand command) =>
        Fields(command).TryGetProperty("id", out var id) ? id.GetString() ?? "" : "";

    public static string Cmd(CoreCommand command) => Fields(command).GetProperty("cmd").GetString() ?? "";

    /// <summary>
    /// The summary the meeting chain stored for the seeded launch meeting (ink-llm's
    /// render_markdown over a summary answer): what the screen must render without a token of
    /// markdown.
    /// </summary>
    public const string SeededSummary = """
        Launch moves to the 14th, after the design review

        ### Launch date

        The design review needs **another week**, so the launch moves to the *fourteenth*.

        ### Beta

        - Keep the beta group small, about forty people
        - Security wants it in writing that the models run on the laptop

        1. Revised plan first
        2. Then the budget sheet

        ## Decisions
        - Move the launch to the fourteenth
        - Keep the beta group to about forty

        ## Actions
        - Send the revised plan and the budget sheet (You, Friday)

        ## Open questions
        - Who signs off the design review?
        """;

    public const string RecordAnswer = """
        {"type":"library.record","ref":"REQ",
         "record":{"record":"r1","kind":"meeting","title":"Launch moves to the 14th","started_at_unix_ms":1790250000000,"ended_at_unix_ms":1790250030000,"source_app":"Zoom","revision":2,"has_audio":true},
         "segments":[
           {"channel":"far","start_ms":0,"end_ms":10000,"text":"The design review needs another week.","speaker":"spk0"},
           {"channel":"mic","start_ms":486,"end_ms":1274,"text":"Let's settle the launch date."},
           {"channel":"mic","start_ms":2822,"end_ms":3546,"text":"I'll send the revised plan by Friday."},
           {"channel":"far","start_ms":10000,"end_ms":13466,"text":"Can you share the budget?","speaker":"spk1"},
           {"channel":"far","start_ms":18278,"end_ms":19258,"text":"Agreed, the fourteenth works.","speaker":"spk2"},
           {"channel":"far","start_ms":20000,"end_ms":21000,"text":"Thanks, all."}],
         "notes":[{"note":"n2","at_ms":15000,"text":"Beta: small group"},{"note":"n1","at_ms":2000,"text":"Launch date"}],
         "summary":{"text":"Launch moves to the 14th\n\nThe review needs **another week**.","model":"scripted/seed","created_at_unix_ms":1790250031000,"items":[]},
         "commitments":[
           {"commitment":"c1","record":"r1","text":"Send the revised plan","owner":"You","due":"Friday","provenance":[{"channel":"mic","start_ms":2822,"end_ms":3546}],"merged_into":"c2","done":false},
           {"commitment":"c2","record":"r1","text":"Send the revised plan","due":"Friday","provenance":[{"channel":"mic","start_ms":2822,"end_ms":3546}],"done":false},
           {"commitment":"c3","record":"r1","text":"Share the budget","provenance":[{"channel":"far","start_ms":10000,"end_ms":13466}],"done":true}],
         "speakers":[{"speaker":"spk0","name":"Alex"}],
         "audio":{"timeline":"recorded","left_out":0,"chunks":[
           {"channel":"mic","path":"/nonexistent/mic-000000-16000x1.pcm","start_ms":3,"frames":480000,"sample_rate":16000,"channels":1,"data_offset":64},
           {"channel":"far","path":"/nonexistent/far-000000-16000x1.pcm","start_ms":3,"frames":480000,"sample_rate":16000,"channels":1,"data_offset":64}]}}
        """;

    public static LibraryRecord RecordEvent(string request = "REQ") =>
        Ev.Of<LibraryRecord>(RecordAnswer.Replace("REQ", request, StringComparison.Ordinal));

    public static RecordDocument Document(string request = "REQ") => new(RecordEvent(request));
}

/// <summary>A search delay the test runs by hand: nothing fires until <see cref="Fire"/>.</summary>
internal sealed class ManualSearchScheduler : ISearchScheduler
{
    private readonly List<Pending> _pending = [];

    public int Waiting => _pending.Count(p => !p.Cancelled);

    public TimeSpan? LastDelay { get; private set; }

    public IDisposable After(TimeSpan delay, Action action)
    {
        LastDelay = delay;
        var pending = new Pending(action);
        _pending.Add(pending);
        return pending;
    }

    /// <summary>The pause has passed: runs what is still waiting.</summary>
    public void Fire()
    {
        var due = _pending.Where(p => !p.Cancelled).ToList();
        _pending.Clear();
        foreach (var p in due)
        {
            p.Action();
        }
    }

    private sealed class Pending(Action action) : IDisposable
    {
        public Action Action { get; } = action;

        public bool Cancelled { get; private set; }

        public void Dispose() => Cancelled = true;
    }
}
