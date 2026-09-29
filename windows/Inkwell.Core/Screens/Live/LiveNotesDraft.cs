// The Live screen's notes, as the Mac's LiveNotesDraft: paragraphs in, note commands out. Each
// paragraph is one note, stamped with where in the meeting it was started, and saved to the record
// (note.add) when the user moves on from it: another line, the editor losing focus, or the meeting
// ending. Edits to a saved line update it; emptying one deletes it. Nothing is saved on a timer.
//
// Every command carries a ref naming the record, the line and the kind of command, and every
// answer (note.added, note.deleted, or command.failed with the ref as its id) is matched back to
// its line. A failed save leaves the line unsaved, to be tried again when the user next moves on;
// a failed delete keeps the note's id, so the line is never added twice.
//
// Editor-agnostic: the view hands over the whole text and the caret's paragraph (the Mac's
// notesEdited(text, caretParagraph:)), whatever editor it uses.
using System.Globalization;

namespace Inkwell.Core.Screens;

/// <summary>One paragraph of the notes and what the core has of it.</summary>
internal sealed class LiveNoteLine(int key, string text)
{
    /// <summary>Local and stable while the line lives.</summary>
    public int Key { get; } = key;
    public string Text { get; set; } = text;
    /// <summary>Where in the meeting it was started, ms: set on its first character.</summary>
    public ulong? AtMs { get; set; }
    /// <summary>The core's id, once added. Kept while a delete is on its way, and after a delete failed.</summary>
    public string? Note { get; set; }
    /// <summary>The text the core has, or has been sent. Cleared when a save fails, so the next save tries again.</summary>
    public string? Saved { get; set; }
    /// <summary>note.add is on its way.</summary>
    public bool Adding { get; set; }
    /// <summary>
    /// note.delete is on its way (the line was emptied). Nothing else is sent for the line until it
    /// is answered: a retype then updates the note (the delete failed) or adds a new one.
    /// </summary>
    public bool Deleting { get; set; }

    public bool IsBlank => string.IsNullOrWhiteSpace(Text);
}

/// <summary>The notes of one live meeting. UI thread only.</summary>
internal sealed class LiveNotesDraft(string record)
{
    /// <summary>What a command for a line is: its ref's suffix.</summary>
    internal enum RefKind
    {
        Add,
        Update,
        Delete,
    }

    private List<LiveNoteLine> lines = [new(0, "")];
    private int nextKey = 1;
    /// <summary>The line with the caret; it is saved when the user moves on.</summary>
    private int? editingKey;
    /// <summary>Notes of lines that no longer exist, by line key: their delete is on its way.</summary>
    private readonly Dictionary<int, string> orphans = [];
    /// <summary>Notes of lines that no longer exist whose delete failed: tried again on the next flush.</summary>
    private readonly Dictionary<int, string> orphansToRetry = [];

    /// <summary>The meeting's record.</summary>
    public string Record { get; } = record;

    public IReadOnlyList<LiveNoteLine> Lines => lines;

    private static string Suffix(RefKind kind) => kind switch
    {
        RefKind.Update => ":update",
        RefKind.Delete => ":delete",
        _ => "",
    };

    /// <summary>The ref of <paramref name="kind"/> for line <paramref name="key"/>: the record, the line, and the kind.</summary>
    public string Ref(int key, RefKind kind = RefKind.Add) =>
        $"{Record}:line:{key.ToString(CultureInfo.InvariantCulture)}{Suffix(kind)}";

    /// <summary>The line key in <paramref name="reference"/> if it is a ref of <paramref name="kind"/> of this draft's record.</summary>
    private int? Key(string reference, RefKind kind)
    {
        var prefix = $"{Record}:line:";
        var suffix = Suffix(kind);
        if (!reference.StartsWith(prefix, StringComparison.Ordinal) || !reference.EndsWith(suffix, StringComparison.Ordinal)
            || reference.Length < prefix.Length + suffix.Length)
        {
            return null;
        }
        var middle = reference[prefix.Length..^suffix.Length];
        return int.TryParse(middle, NumberStyles.None, CultureInfo.InvariantCulture, out var key) ? key : null;
    }

    private int IndexOf(int key) => lines.FindIndex(l => l.Key == key);

    /// <summary>The editor's paragraphs after an edit, with the caret's paragraph (null: no caret). Returns what to send.</summary>
    public List<CoreCommand> Edited(IReadOnlyList<string> paragraphs, int? caret, ulong nowMs)
    {
        var commands = Align(paragraphs.Count == 0 ? [""] : paragraphs);
        foreach (var line in lines.Where(l => l.AtMs is null && !l.IsBlank))
        {
            line.AtMs = nowMs;
        }
        editingKey = caret is int c && c >= 0 && c < lines.Count ? lines[c].Key : null;
        for (var i = 0; i < lines.Count; i++)
        {
            if (lines[i].Key != editingKey)
            {
                commands.AddRange(Persist(i));
            }
        }
        return commands;
    }

    /// <summary>
    /// Saves every line, the one being edited included (focus left, the meeting ended, or the app
    /// quitting), and tries again the deletes that failed.
    /// </summary>
    public List<CoreCommand> Flush()
    {
        editingKey = null;
        var commands = new List<CoreCommand>();
        for (var i = 0; i < lines.Count; i++)
        {
            commands.AddRange(Persist(i));
        }
        foreach (var (key, note) in orphansToRetry.OrderBy(o => o.Key))
        {
            orphans[key] = note;
            commands.Add(new CoreCommand.NoteDelete(note, Ref(key, RefKind.Delete)));
        }
        orphansToRetry.Clear();
        return commands;
    }

    /// <summary>The core added the note sent for <paramref name="reference"/>.</summary>
    public List<CoreCommand> Added(string reference, string note)
    {
        if (Key(reference, RefKind.Add) is not int key)
        {
            return [];
        }
        var i = IndexOf(key);
        if (i < 0)
        {
            // Its line was deleted while the add was on its way.
            orphans[key] = note;
            return [new CoreCommand.NoteDelete(note, Ref(key, RefKind.Delete))];
        }
        lines[i].Note = note;
        lines[i].Adding = false;
        return lines[i].Key == editingKey ? [] : Persist(i);
    }

    /// <summary>The core refused an add, update or delete: the line is rolled back to what the core has.</summary>
    public List<CoreCommand> Failed(string reference)
    {
        if (Key(reference, RefKind.Add) is int addKey && IndexOf(addKey) is var a and >= 0)
        {
            // No note: tried again on the next save.
            lines[a].Adding = false;
            lines[a].Saved = null;
        }
        else if (Key(reference, RefKind.Update) is int updateKey && IndexOf(updateKey) is var u and >= 0)
        {
            // The core has the older text: tried again on the next save.
            lines[u].Saved = null;
        }
        else if (Key(reference, RefKind.Delete) is int deleteKey)
        {
            var d = IndexOf(deleteKey);
            if (d >= 0)
            {
                // The note is still there: a retyped line updates it, and a still-empty line
                // deletes it again on the next save.
                lines[d].Deleting = false;
                lines[d].Saved = null;
                if (!lines[d].IsBlank && lines[d].Key != editingKey)
                {
                    return Persist(d);
                }
            }
            else if (orphans.Remove(deleteKey, out var note))
            {
                orphansToRetry[deleteKey] = note;
            }
        }
        return [];
    }

    /// <summary>The core deleted the note sent for <paramref name="reference"/>.</summary>
    public List<CoreCommand> Deleted(string reference)
    {
        if (Key(reference, RefKind.Delete) is not int key)
        {
            return [];
        }
        var i = IndexOf(key);
        if (i < 0)
        {
            orphans.Remove(key);
            return [];
        }
        lines[i].Deleting = false;
        lines[i].Note = null;
        lines[i].Saved = null;
        // Retyped while the delete was on its way: it is a new note now.
        return !lines[i].IsBlank && lines[i].Key != editingKey ? Persist(i) : [];
    }

    /// <summary>
    /// Matches the old lines to <paramref name="paragraphs"/>: unchanged lines at both ends keep
    /// their identity, and the changed stretch between them is paired up in order, with extra
    /// paragraphs becoming new lines and extra lines deleted. One edit changes one stretch, so a
    /// line keeps its identity (and its note) while it is typed in, split or joined.
    /// </summary>
    private List<CoreCommand> Align(IReadOnlyList<string> paragraphs)
    {
        var old = lines.Select(l => l.Text).ToList();
        var head = 0;
        while (head < old.Count && head < paragraphs.Count && old[head] == paragraphs[head])
        {
            head++;
        }
        var tail = 0;
        while (tail < old.Count - head && tail < paragraphs.Count - head
               && old[old.Count - 1 - tail] == paragraphs[paragraphs.Count - 1 - tail])
        {
            tail++;
        }
        var oldMiddle = lines.GetRange(head, old.Count - tail - head);
        var newMiddle = paragraphs.Skip(head).Take(paragraphs.Count - tail - head).ToList();
        var middle = new List<LiveNoteLine>();
        var commands = new List<CoreCommand>();
        for (var i = 0; i < newMiddle.Count; i++)
        {
            if (i < oldMiddle.Count)
            {
                oldMiddle[i].Text = newMiddle[i];
                middle.Add(oldMiddle[i]);
            }
            else
            {
                middle.Add(new LiveNoteLine(nextKey, newMiddle[i]));
                nextKey++;
            }
        }
        foreach (var line in oldMiddle.Skip(newMiddle.Count))
        {
            if (line.Note is not string note)
            {
                // A line whose add is on its way is deleted when note.added names it (Added).
                continue;
            }
            orphans[line.Key] = note;
            if (!line.Deleting)
            {
                commands.Add(new CoreCommand.NoteDelete(note, Ref(line.Key, RefKind.Delete)));
            }
        }
        lines = [.. lines.Take(head), .. middle, .. lines.Skip(old.Count - tail)];
        return commands;
    }

    /// <summary>What saving line <paramref name="i"/> takes now.</summary>
    private List<CoreCommand> Persist(int i)
    {
        var line = lines[i];
        if (line.Deleting)
        {
            return [];
        }
        if (line.IsBlank)
        {
            if (line.Note is not string note)
            {
                return [];
            }
            line.Deleting = true;
            line.Saved = null;
            return [new CoreCommand.NoteDelete(note, Ref(line.Key, RefKind.Delete))];
        }
        if (line.Saved == line.Text)
        {
            return [];
        }
        if (line.Note is string existing)
        {
            line.Saved = line.Text;
            return [new CoreCommand.NoteUpdate(existing, line.Text, Ref(line.Key, RefKind.Update))];
        }
        if (line.Adding)
        {
            return [];
        }
        line.Adding = true;
        line.Saved = line.Text;
        return [new CoreCommand.NoteAdd(Record, line.AtMs ?? 0, line.Text, Ref(line.Key))];
    }
}
