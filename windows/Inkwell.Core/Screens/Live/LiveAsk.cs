// Ask, with the question stack, as the Mac's QuestionStack (FarEndQuestions here: a type name may
// not end in "Stack", CA1711) and AskAnswer: the far end's questions (up to four, newest first) are
// stacked so one can be answered with a key press, and anything else can be asked. The core answers
// (meeting.ask) from the transcript so far, with the language model the shell registered; the
// answer is the model's words, shown as plain text only, and a failure says so in words.
using System.Collections.Immutable;
using System.Globalization;
using System.Text;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>A question the far end asked.</summary>
public sealed record StackedQuestion(int Id, string Text, long AtMs);

/// <summary>The far end's questions, newest first, at most four.</summary>
public sealed class FarEndQuestions
{
    public const int Capacity = 4;
    /// <summary>Shorter questions ("Right?") are not worth a slot.</summary>
    public const int MinimumLength = 12;

    private readonly HashSet<string> seen = [];
    private int nextId;

    public ImmutableList<StackedQuestion> Questions { get; private set; } = [];

    /// <summary>The key that answers the question in <paramref name="slot"/> (Ctrl+1 is the newest; the Mac's ⌘1).</summary>
    public static string KeyLabel(int slot) => string.Create(CultureInfo.InvariantCulture, $"Ctrl+{slot + 1}");

    /// <summary>
    /// A final from the far end: its questions go on the stack. The user's own questions do not:
    /// what is asked of them is what they may want help with.
    /// </summary>
    public void Heard(MeetingFinal final)
    {
        ArgumentNullException.ThrowIfNull(final);
        if (final.Channel != Channel.Far)
        {
            return;
        }
        foreach (var sentence in Sentences(final.Text).Where(s => s.EndsWith('?')))
        {
            // Characters as the user sees them, the question mark aside.
            if (new StringInfo(sentence).LengthInTextElements - 1 < MinimumLength)
            {
                continue;
            }
            if (!seen.Add(sentence.ToLowerInvariant()))
            {
                continue;
            }
            Questions = Questions.Insert(0, new StackedQuestion(nextId, sentence, final.StartMs));
            nextId++;
        }
        if (Questions.Count > Capacity)
        {
            Questions = Questions.RemoveRange(Capacity, Questions.Count - Capacity);
        }
    }

    /// <summary>Sentences, each with its end mark.</summary>
    public static IReadOnlyList<string> Sentences(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        var sentences = new List<string>();
        var current = new StringBuilder();
        foreach (var ch in text)
        {
            current.Append(ch);
            if (ch is '?' or '.' or '!')
            {
                var s = current.ToString().Trim();
                if (s.Length > 0)
                {
                    sentences.Add(s);
                }
                current.Clear();
            }
        }
        var rest = current.ToString().Trim();
        if (rest.Length > 0)
        {
            sentences.Add(rest);
        }
        return sentences;
    }
}

/// <summary>What Ask answered.</summary>
public abstract record AskAnswer
{
    private AskAnswer() { }

    /// <summary>The model's words: shown as plain text only (never parsed, no link in it opens anything).</summary>
    public sealed record Answer(string Text) : AskAnswer;

    /// <summary>Nothing answered; why, in the user's words.</summary>
    public sealed record Unavailable(string Why) : AskAnswer;

    /// <summary>
    /// What the core's failure means for the user. Its message names what failed (never the
    /// question); the cases without a model, without the user's OK, and after the meeting get
    /// their own words.
    /// </summary>
    public static AskAnswer Failed(string message)
    {
        ArgumentNullException.ThrowIfNull(message);
        // The core's NEEDS_CONSENT (ink-ffi asking.rs): nothing was sent. Its start is stable.
        if (message.StartsWith("Ask needs your OK", StringComparison.Ordinal))
        {
            return new Unavailable("Ask needs your OK first: turn on Summaries and Ask in Settings > AI.");
        }
        if (message.Contains("no language model", StringComparison.Ordinal))
        {
            // The Mac names Apple Intelligence; on Windows the model is one the core's engines
            // registered (S3.2).
            return new Unavailable("Answers need a language model, which is off or not ready on this PC.");
        }
        if (message.Contains("no meeting", StringComparison.Ordinal))
        {
            return new Unavailable("Couldn't answer: the meeting has ended.");
        }
        return new Unavailable("Couldn't answer that. Try asking again.");
    }
}

/// <summary>A question the user asked, and its answer (null while waiting).</summary>
public sealed record AskedQuestion(int Id, string Question, AskAnswer? Answer = null)
{
    /// <summary>What shows under the question: "Thinking…", the answer, or why there is none.</summary>
    public string AnswerText => Answer switch
    {
        null => "Thinking…",
        AskAnswer.Answer a => a.Text,
        AskAnswer.Unavailable u => u.Why,
        _ => "",
    };

    /// <summary>The model's words (the view shows them in the text colour; the rest is secondary).</summary>
    public bool IsAnswer => Answer is AskAnswer.Answer;
}
