// The commands the screens send (inkwell.h lists them), as the Mac's CoreCommand. A view model
// sends these through a delegate, so a test can read what it sent and answer with events of its
// own. Records: two commands are equal when their fields are.
using System.Buffers;
using System.Text;
using System.Text.Json;
using Inkwell.Core.Events;

namespace Inkwell.Core.Screens;

/// <summary>One command to the core.</summary>
public abstract record CoreCommand
{
    private CoreCommand() { }

    /// <summary>The command's name, for a log line (never its fields: a note's words travel in them).</summary>
    public abstract string Name { get; }

    /// <summary>Its fields, "cmd" among them.</summary>
    private protected abstract IEnumerable<(string Key, object Value)> Fields();

    /// <summary>Its JSON, as ink_command reads it (keys sorted, so the same command reads the same).</summary>
    public string Json => JsonFields.Write(Fields());

    /// <summary>Its "id", when it carries one (answers echo it as ref; a command.failed as its id).</summary>
    public string? CommandId => Fields().Where(f => f.Key == "id").Select(f => f.Value as string).FirstOrDefault();

    /// <summary>
    /// The command.failed the core would have sent had it run this command and failed: the shell
    /// raises it when the command never reached the core (no core, or the core refused to queue
    /// it), so the screen that waits for an answer says "couldn't" instead of waiting forever.
    /// <paramref name="message"/> names what went wrong, never the command's fields.
    /// </summary>
    public CommandFailed NotSent(string message) =>
        new() { Type = "command.failed", Command = Name, Id = CommandId, Message = message };

    public sealed record PermissionsCheck : CoreCommand
    {
        public override string Name => "permissions.check";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name)];
    }

    public sealed record PermissionRequest(PermissionName Permission) : CoreCommand
    {
        public override string Name => "permission.request";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("permission", Wire.Name(Permission))];
    }

    public sealed record CommitmentsList : CoreCommand
    {
        public override string Name => "commitments.list";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name)];
    }

    public sealed record CommitmentSetDone(string Id, bool Done) : CoreCommand
    {
        public override string Name => "commitment.set_done";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("commitment", Id), ("done", Done)];
    }

    /// <summary><paramref name="Ref"/> comes back in note.added, so the note can be matched to the line that sent it.</summary>
    public sealed record NoteAdd(string Record, ulong AtMs, string Text, string Ref) : CoreCommand
    {
        public override string Name => "note.add";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("record", Record), ("at_ms", AtMs), ("text", Text), ("id", Ref)];
    }

    /// <summary><paramref name="Ref"/> comes back in note.updated, or as the id of a command.failed.</summary>
    public sealed record NoteUpdate(string Note, string Text, string Ref) : CoreCommand
    {
        public override string Name => "note.update";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("note", Note), ("text", Text), ("id", Ref)];
    }

    /// <summary><paramref name="Ref"/> comes back in note.deleted, or as the id of a command.failed.</summary>
    public sealed record NoteDelete(string Note, string Ref) : CoreCommand
    {
        public override string Name => "note.delete";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("note", Note), ("id", Ref)];
    }

    public sealed record ModelsList : CoreCommand
    {
        public override string Name => "models.list";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name)];
    }

    /// <summary>
    /// Installs <paramref name="Next"/> in place of <paramref name="Model"/>: with both the same
    /// registry id, the first download of that model (nothing else is unloaded or warmed). Only
    /// when the user asks for it. Its id names the model ("model.update:&lt;next&gt;"), so a failure
    /// is matched to its row.
    /// </summary>
    public sealed record ModelUpdate(string Model, string Next) : CoreCommand
    {
        public override string Name => "model.update";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("model", Model), ("next", Next), ("id", $"{Name}:{Next}")];
    }

    /// <summary>
    /// Stops <paramref name="Model"/>'s download (one running or queued in the core), only when the
    /// user asks: it ends with that update's model.update_finished, cancelled, and its part files
    /// stay for the next download to resume. Its id names the model ("model.cancel:&lt;id&gt;"), so a
    /// failure (not_downloading) is matched to its row.
    /// </summary>
    public sealed record ModelCancel(string Model) : CoreCommand
    {
        public override string Name => "model.cancel";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("model", Model), ("id", $"{Name}:{Model}")];
    }

    /// <summary>
    /// Deletes <paramref name="Model"/>'s files, only after the user confirmed it: models.listed
    /// with the id as its ref, or a command.failed with it (model_in_use: nothing was deleted). Its
    /// id names the model ("model.remove:&lt;id&gt;").
    /// </summary>
    public sealed record ModelRemove(string Model) : CoreCommand
    {
        public override string Name => "model.remove";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("model", Model), ("id", $"{Name}:{Model}")];
    }

    /// <summary>
    /// Loads the job's model and keeps it loaded: answered by model.warmed, model.refused or
    /// model.warm_failed. Its id names the job ("model.warm:dictation_final"), as the Mac's.
    /// </summary>
    public sealed record ModelWarm(Job Job) : CoreCommand
    {
        public override string Name => "model.warm";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("job", Wire.Name(Job)), ("id", $"{Name}:{Wire.Name(Job)}")];
    }

    /// <summary>Its id names the job ("engine.route:dictation_final"), so a failure is matched to its line.</summary>
    public sealed record EngineRoute(Job Job) : CoreCommand
    {
        public override string Name => "engine.route";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("job", Wire.Name(Job)), ("id", $"{Name}:{Wire.Name(Job)}")];
    }

    /// <summary>The id names the setting, so a failure can be matched to it (command.failed has no key).</summary>
    public sealed record SettingGet(ShellSetting Key) : CoreCommand
    {
        public override string Name => "setting.get";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("key", Key.Key()), ("id", Key.CommandId())];
    }

    public sealed record SettingSet(ShellSetting Key, string Value) : CoreCommand
    {
        public override string Name => "setting.set";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("key", Key.Key()), ("value", Value), ("id", Key.CommandId())];
    }

    /// <summary>
    /// Settings > Modes: each answered by modes.listed with <paramref name="Ref"/> (a save's with
    /// saved), or a command.failed with it as the id and, for a refusal the editor shows, a code.
    /// </summary>
    public sealed record ModesList(string Ref) : CoreCommand
    {
        public override string Name => "modes.list";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    /// <summary>A mode added or changed: only the fields <paramref name="Save"/> sets are named.</summary>
    public sealed record ModesSave(ModeSave Save, string Ref) : CoreCommand
    {
        public override string Name => "modes.save";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("mode", Save.ModeFields());
            yield return ("id", Ref);
            if (Save.TakeApps)
            {
                yield return ("take_apps", true);
            }
            if (Save.ReplaceUnreadable)
            {
                yield return ("replace_unreadable", true);
            }
        }
    }

    /// <summary>A mode deleted, only after the user confirmed it: its apps go back to the default mode.</summary>
    public sealed record ModesDelete(string Mode, string Ref) : CoreCommand
    {
        public override string Name => "modes.delete";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("mode", Mode), ("id", Ref)];
    }

    /// <summary>
    /// The library (Today, Library, a record). <paramref name="Ref"/> comes back as the answer's
    /// ref, or as the id of a command.failed, so a model matches each answer to its question and
    /// can tell "could not load" from "empty".
    /// </summary>
    public sealed record RecordsList(RecordKind? Kind, RecordCursor? Before, int Limit, string Ref) : CoreCommand
    {
        public override string Name => "records.list";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("limit", Limit);
            yield return ("id", Ref);
            if (Kind is RecordKind kind)
            {
                yield return ("kind", Wire.Name(kind));
            }
            if (Before is RecordCursor before)
            {
                yield return ("before", new SortedDictionary<string, object>(StringComparer.Ordinal)
                {
                    ["started_at_unix_ms"] = before.StartedAtUnixMs,
                    ["id"] = before.Record,
                });
            }
        }
    }

    public sealed record RecordsSearch(string Query, int Limit, string Ref) : CoreCommand
    {
        public override string Name => "records.search";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("query", Query), ("limit", Limit), ("id", Ref)];
    }

    public sealed record RecordOpen(string Record, string Ref) : CoreCommand
    {
        public override string Name => "record.open";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("record", Record), ("id", Ref)];
    }

    /// <summary>
    /// Names a far-end speaker of a record by the diarizer's label (<paramref name="Given"/> is the
    /// name; empty clears it). Answered by speaker.named with the ref, or a command.failed with it
    /// as the id.
    /// </summary>
    public sealed record SpeakerName(string Record, string Speaker, string Given, string Ref) : CoreCommand
    {
        public override string Name => "speaker.name";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("record", Record), ("speaker", Speaker), ("name", Given), ("id", Ref)];
    }

    /// <summary>
    /// Asks whether this computer can watch <paramref name="Binding"/> as a dictation or edit key
    /// (the shortcut recorder, before it stores one). Answered by hotkey.checked with the ref: its
    /// canonical spelling, or why not; or a command.failed with it as the id.
    /// </summary>
    public sealed record HotkeyCheck(string Binding, string Ref) : CoreCommand
    {
        public override string Name => "hotkey.check";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("binding", Binding), ("id", Ref)];
    }

    /// <summary>
    /// Deletes a record whole, only after the user confirmed it. Answered by record.deleted with
    /// the ref, or a command.failed with it as the id (a record still live is refused).
    /// </summary>
    public sealed record RecordDelete(string Record, string Ref) : CoreCommand
    {
        public override string Name => "record.delete";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("record", Record), ("id", Ref)];
    }

    public sealed record LibraryStats(long SinceUnixMs, string Ref) : CoreCommand
    {
        public override string Name => "library.stats";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("since_unix_ms", SinceUnixMs), ("id", Ref)];
    }

    /// <summary>A start names the app when it answers an offer, and a title when the calendar has the call.</summary>
    public sealed record MeetingStart(string? App, string? Title) : CoreCommand
    {
        public override string Name => "meeting.start";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("id", Name);
            if (App is not null)
            {
                yield return ("app", App);
            }
            if (Title is not null)
            {
                yield return ("title", Title);
            }
        }
    }

    public sealed record MeetingStop : CoreCommand
    {
        public override string Name => "meeting.stop";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Name)];
    }

    public sealed record MeetingDismiss(string App) : CoreCommand
    {
        public override string Name => "meeting.dismiss";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("app", App), ("id", Name)];
    }

    /// <summary>
    /// "Stop and delete", in a meeting's first minute (until its delete_until_unix_ms): the
    /// recording ends and is deleted as if never made (meeting.stopped, then meeting.discarded).
    /// </summary>
    public sealed record MeetingDiscard : CoreCommand
    {
        public override string Name => "meeting.discard";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Name)];
    }

    /// <summary>The call policies: meetings.calls with <paramref name="Ref"/>, or a command.failed with it as the id.</summary>
    public sealed record MeetingsCallsList(string Ref) : CoreCommand
    {
        public override string Name => "meetings.calls.list";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    /// <summary>
    /// One app's call policy, by the identity detection reports: always, ask, never, or default
    /// (follow the default again). The core refuses it over a stored list it cannot read unless
    /// <paramref name="ReplaceUnreadable"/> (the user chose to start the list over).
    /// </summary>
    public sealed record MeetingsCallsSet(string App, string Policy, bool ReplaceUnreadable, string Ref) : CoreCommand
    {
        public override string Name => "meetings.calls.set";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("app", App);
            yield return ("policy", Policy);
            yield return ("id", Ref);
            if (ReplaceUnreadable)
            {
                yield return ("replace_unreadable", true);
            }
        }
    }

    /// <summary><paramref name="Ref"/> comes back in meeting.answered, or as the id of a command.failed.</summary>
    public sealed record MeetingAsk(string Question, string Ref) : CoreCommand
    {
        public override string Name => "meeting.ask";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("question", Question), ("id", Ref)];
    }

    public sealed record MeetingsRecover : CoreCommand
    {
        public override string Name => "meetings.recover";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name)];
    }

    /// <summary>"Not yet": a looks-done suggestion is dismissed.</summary>
    public sealed record CommitmentNotYet(string Id) : CoreCommand
    {
        public override string Name => "commitment.not_yet";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("commitment", Id)];
    }

    /// <summary>
    /// Dictation live (the core holds the keys): answered by dictation.ready or dictation.off with
    /// <paramref name="Ref"/>. <paramref name="UtcOffsetMinutes"/> is for {date} and {time} in snippets.
    /// </summary>
    public sealed record DictationEnable(int UtcOffsetMinutes, string Ref) : CoreCommand
    {
        public override string Name => "dictation.enable";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("utc_offset_minutes", UtcOffsetMinutes), ("id", Ref)];
    }

    /// <summary>Lets go of the keys and the mic: dictation.off with <paramref name="Ref"/>.</summary>
    public sealed record DictationDisable(string Ref) : CoreCommand
    {
        public override string Name => "dictation.disable";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    /// <summary>
    /// A feature's switch, where it would send now, and the user's consent: consent.state with
    /// <paramref name="Ref"/>, or command.failed with it as the id.
    /// </summary>
    public sealed record ConsentGet(LlmFeature Feature, string Ref) : CoreCommand
    {
        public override string Name => "consent.get";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("feature", Wire.Name(Feature)), ("id", Ref)];
    }

    /// <summary>
    /// The user agreed, in the consent step, that the feature may send to <paramref name="To"/> (for
    /// a cloud model, the endpoint consent.state named; for voice edit, with its key): the core
    /// records it and turns the feature on, or fails if the model has moved since.
    /// </summary>
    public sealed record ConsentAllow(LlmFeature Feature, LlmDestination To, string? Endpoint, string? Key, string Ref) : CoreCommand
    {
        public override string Name => "consent.allow";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("feature", Wire.Name(Feature));
            yield return ("to", Wire.Name(To));
            yield return ("id", Ref);
            if (Endpoint is not null)
            {
                yield return ("endpoint", Endpoint);
            }
            if (Key is not null)
            {
                yield return ("key", Key);
            }
        }
    }

    /// <summary>
    /// Takes polish's consent for one destination away (Settings > AI lists each): consent.state
    /// with <paramref name="Ref"/>, or command.failed with it as the id. Revoking the last turns
    /// polish off.
    /// </summary>
    public sealed record ConsentRevoke(LlmFeature Feature, LlmDestination To, string? Endpoint, string Ref) : CoreCommand
    {
        public override string Name => "consent.revoke";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("feature", Wire.Name(Feature));
            yield return ("to", Wire.Name(To));
            yield return ("id", Ref);
            if (Endpoint is not null)
            {
                yield return ("endpoint", Endpoint);
            }
        }
    }

    /// <summary>Settings > AI's language model: the own-key providers and the one chosen (llm.providers with <paramref name="Ref"/>).</summary>
    public sealed record LlmProviders(string Ref) : CoreCommand
    {
        public override string Name => "llm.providers";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    /// <summary>
    /// Stores <paramref name="Key"/> for <paramref name="Provider"/> in the OS key store (the core
    /// keeps it nowhere else). Its ToString never shows the key: only Json carries it, once, to the core.
    /// </summary>
    public sealed record LlmKeySave(string Provider, string Key, string Ref) : CoreCommand
    {
        public override string Name => "llm.key.save";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("provider", Provider), ("key", Key), ("id", Ref)];

        public override string ToString() => $"LlmKeySave {{ Provider = {Provider}, Key = <redacted>, Ref = {Ref} }}";
    }

    public sealed record LlmKeyDelete(string Provider, string Ref) : CoreCommand
    {
        public override string Name => "llm.key.delete";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("provider", Provider), ("id", Ref)];
    }

    /// <summary>
    /// Chooses <paramref name="Provider"/> ("none": none) and its model. <paramref name="LocalOnlyOff"/>
    /// is the user's say-so for a provider that is not on this PC: choosing it turns local-only
    /// mode off, and the core refuses such a choice without it.
    /// </summary>
    public sealed record LlmChoose(string Provider, string? Model, string? BaseUrl, bool LocalOnlyOff, string Ref) : CoreCommand
    {
        public override string Name => "llm.choose";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("provider", Provider);
            yield return ("id", Ref);
            if (Model is not null)
            {
                yield return ("model", Model);
            }
            if (BaseUrl is not null)
            {
                yield return ("base_url", BaseUrl);
            }
            if (LocalOnlyOff)
            {
                yield return ("local_only", "off");
            }
        }
    }

    /// <summary>One short fixed request to the chosen provider: llm.tested with <paramref name="Ref"/>.</summary>
    public sealed record LlmTest(string Ref) : CoreCommand
    {
        public override string Name => "llm.test";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    /// <summary>
    /// The Stats screen's numbers (stats.counted with <paramref name="Ref"/>, or a command.failed
    /// with it as the id), counted on the user's calendar: their zone's UTC offsets over time and
    /// the ISO weekday their weeks start on (StatsModel.CalendarFields).
    /// </summary>
    public sealed record StatsGet(IReadOnlyList<UtcOffset> UtcOffsets, int WeekStart, string Ref) : CoreCommand
    {
        public override string Name => "stats.get";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("utc_offsets", UtcOffsets.Select(o => o.Fields()).ToList()), ("week_start", WeekStart), ("id", Ref)];
    }

    /// <summary>The milestones reached since the last check, each reported once ever: milestones.reached with <paramref name="Ref"/>. Takes stats.get's calendar.</summary>
    public sealed record MilestonesCheck(IReadOnlyList<UtcOffset> UtcOffsets, int WeekStart, string Ref) : CoreCommand
    {
        public override string Name => "milestones.check";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("utc_offsets", UtcOffsets.Select(o => o.Fields()).ToList()), ("week_start", WeekStart), ("id", Ref)];
    }

    /// <summary>
    /// Pauses the streak from today (days without a dictation then don't count against it, for up
    /// to 90 days): stats.counted with <paramref name="Ref"/>, or a command.failed with it as the
    /// id. Takes stats.get's calendar.
    /// </summary>
    public sealed record StreakPause(IReadOnlyList<UtcOffset> UtcOffsets, int WeekStart, string Ref) : CoreCommand
    {
        public override string Name => "streak.pause";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("utc_offsets", UtcOffsets.Select(o => o.Fields()).ToList()), ("week_start", WeekStart), ("id", Ref)];
    }

    /// <summary>Ends the running pause of the streak: answered as <see cref="StreakPause"/>.</summary>
    public sealed record StreakResume(IReadOnlyList<UtcOffset> UtcOffsets, int WeekStart, string Ref) : CoreCommand
    {
        public override string Name => "streak.resume";
        private protected override IEnumerable<(string, object)> Fields() =>
            [("cmd", Name), ("utc_offsets", UtcOffsets.Select(o => o.Fields()).ToList()), ("week_start", WeekStart), ("id", Ref)];
    }

    /// <summary>Settings > Snippets: answered by snippets.listed with <paramref name="Ref"/>.</summary>
    public sealed record SnippetsList(string Ref) : CoreCommand
    {
        public override string Name => "snippets.list";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    /// <summary>
    /// A save sends the whole list; the core refuses it over a stored list it cannot read unless
    /// <paramref name="ReplaceUnreadable"/> (the user chose to start over).
    /// </summary>
    public sealed record SnippetsSave(IReadOnlyList<SnippetDraft> Snippets, bool ReplaceUnreadable, string Ref) : CoreCommand
    {
        public override string Name => "snippets.save";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("snippets", Snippets.Select(s => s.Fields()).ToList());
            yield return ("id", Ref);
            if (ReplaceUnreadable)
            {
                yield return ("replace_unreadable", true);
            }
        }

        public bool Equals(SnippetsSave? other) =>
            other is not null && Snippets.SequenceEqual(other.Snippets) && ReplaceUnreadable == other.ReplaceUnreadable && Ref == other.Ref;

        public override int GetHashCode() => HashCode.Combine(Snippets.Count, ReplaceUnreadable, Ref);
    }

    /// <summary>Settings > Voice commands: answered by voice_commands.listed with <paramref name="Ref"/>.</summary>
    public sealed record VoiceCommandsList(string Ref) : CoreCommand
    {
        public override string Name => "voice_commands.list";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    public sealed record VoiceCommandsSave(
        bool Enabled, string WakePrefix, IReadOnlyList<VoiceCommandDraft> Commands, bool ReplaceUnreadable, string Ref) : CoreCommand
    {
        public override string Name => "voice_commands.save";
        private protected override IEnumerable<(string, object)> Fields()
        {
            yield return ("cmd", Name);
            yield return ("enabled", Enabled);
            yield return ("wake_prefix", WakePrefix);
            yield return ("commands", Commands.Select(c => c.Fields()).ToList());
            yield return ("id", Ref);
            if (ReplaceUnreadable)
            {
                yield return ("replace_unreadable", true);
            }
        }

        public bool Equals(VoiceCommandsSave? other) =>
            other is not null && Enabled == other.Enabled && WakePrefix == other.WakePrefix
            && Commands.SequenceEqual(other.Commands) && ReplaceUnreadable == other.ReplaceUnreadable && Ref == other.Ref;

        public override int GetHashCode() => HashCode.Combine(Enabled, WakePrefix, Commands.Count, ReplaceUnreadable, Ref);
    }

    /// <summary>What the Inkwell 0.2 import has to say about the dictation key: import.notes.</summary>
    public sealed record ImportNotes : CoreCommand
    {
        public override string Name => "import.notes";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Name)];
    }

    /// <summary>
    /// Whether Inkwell 0.2's data is on this PC and not yet imported (the core knows where it is):
    /// import.checked, or a command.failed, with the command's name as its id.
    /// </summary>
    public sealed record ImportCheck : CoreCommand
    {
        public override string Name => "import.check";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Name)];
    }

    /// <summary>Imports it: import.finished, or a command.failed whose message is words to show.</summary>
    public sealed record ImportRun : CoreCommand
    {
        public override string Name => "import.run";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Name)];
    }

    /// <summary>
    /// Settings > Sound: the microphones, the outputs, the choices and what records now, answered by
    /// audio.devices with <paramref name="Ref"/> (then audio.devices_changed unasked as devices come
    /// and go), or a command.failed with it as the id.
    /// </summary>
    public sealed record AudioDevices(string Ref) : CoreCommand
    {
        public override string Name => "audio.devices";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    /// <summary>
    /// The mic test: audio.test_started, audio.test_level about ten times a second, then
    /// audio.tested, all with <paramref name="Ref"/>; or a command.failed with it as the id
    /// (meeting_recording while a meeting records). Nothing it hears is kept.
    /// </summary>
    public sealed record AudioTest(string Ref) : CoreCommand
    {
        public override string Name => "audio.test";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }

    /// <summary>Ends the running test: its own audio.tested (stopped), or a command.failed with <paramref name="Ref"/> when none runs.</summary>
    public sealed record AudioTestStop(string Ref) : CoreCommand
    {
        public override string Name => "audio.test_stop";
        private protected override IEnumerable<(string, object)> Fields() => [("cmd", Name), ("id", Ref)];
    }
}

/// <summary>Where a page of records continues: the last record of the previous page.</summary>
public readonly record struct RecordCursor(long StartedAtUnixMs, string Record);

/// <summary>A snippet as Settings edits it.</summary>
/// <summary>A UTC offset from the moment it took effect (stats.get's and milestones.check's utc_offsets).</summary>
public sealed record UtcOffset(long FromUnixMs, int Minutes)
{
    internal SortedDictionary<string, object> Fields() => new(StringComparer.Ordinal)
    {
        ["from_unix_ms"] = FromUnixMs,
        ["minutes"] = Minutes,
    };
}

public sealed record SnippetDraft(string Id, string Trigger, string Expansion, string Category = "", bool Enabled = true)
{
    public SnippetDraft(SnippetInfo info)
        : this(info.Id, info.Trigger, info.Expansion, info.Category, info.Enabled)
    {
    }

    /// <summary>Its JSON in snippets.save.</summary>
    internal SortedDictionary<string, object> Fields() => new(StringComparer.Ordinal)
    {
        ["id"] = Id,
        ["trigger"] = Trigger,
        ["expansion"] = Expansion,
        ["category"] = Category,
        ["enabled"] = Enabled,
    };
}

/// <summary>A voice command as Settings edits it.</summary>
/// <param name="CarriedOut">Whether this build carries it out (the core says; a new one is always of a kind it does).</param>
public sealed record VoiceCommandDraft(
    string Id, IReadOnlyList<string> Triggers, CommandAction Action, string? Value, bool Enabled = true, bool CarriedOut = true)
{
    public VoiceCommandDraft(VoiceCommandInfo info)
        : this(info.Id, info.Triggers, info.Action, info.Value, info.Enabled, info.CarriedOut)
    {
    }

    public bool Equals(VoiceCommandDraft? other) =>
        other is not null && Id == other.Id && Triggers.SequenceEqual(other.Triggers) && Action == other.Action
        && Value == other.Value && Enabled == other.Enabled && CarriedOut == other.CarriedOut;

    public override int GetHashCode() => HashCode.Combine(Id, Triggers.Count, Action, Value, Enabled, CarriedOut);

    /// <summary>Its JSON in voice_commands.save.</summary>
    internal SortedDictionary<string, object> Fields()
    {
        var fields = new SortedDictionary<string, object>(StringComparer.Ordinal)
        {
            ["id"] = Id,
            ["triggers"] = Triggers.ToList(),
            ["action"] = Wire.Name(Action),
            ["enabled"] = Enabled,
        };
        if (Value is not null)
        {
            fields["value"] = Value;
        }
        return fields;
    }
}

/// <summary>The settings the shell owns in the core's store (setting.get / setting.set).</summary>
public enum ShellSetting
{
    /// <summary>"true" once the first-run state was completed or skipped.</summary>
    OnboardingDone,
    /// <summary>"on" or "off": the user's switch for dictation polish. Only "off" is set this way: polish turns on through the consent step.</summary>
    DictationPolish,
    /// <summary>
    /// "ask" (the default), "always" or "never": the call policy for apps not chosen for. It
    /// replaced "meetings.detect" ("Offer to record calls"), which the core migrates.
    /// </summary>
    MeetingsCallsDefault,
    /// <summary>"on" or "off": the switch for a meeting's summary and Ask. Only "off" is set this way.</summary>
    MeetingsLlm,
    /// <summary>The microphone for dictation, meetings and the test: "auto" (the default) or a device's id from audio.devices, connected when set.</summary>
    AudioInput,
    /// <summary>The output Record now's far end records: "default" (the default) or an output's id from audio.devices, connected when set.</summary>
    AudioOutput,
    /// <summary>"forever" (the default), or days: how long the library keeps records.</summary>
    RetentionDays,
    /// <summary>The dictation key (a token).</summary>
    DictationKey,
    /// <summary>The voice-edit key, or "off" (which withdraws its consent). Turned on through the consent step.</summary>
    DictationEditKey,
    /// <summary>"on" or "off": whether dictation is live (Settings > Voice).</summary>
    DictationEnabled,
    /// <summary>"dismissed": the note about Inkwell 0.2's dictation key has been read.</summary>
    ImportKeyNote,
    /// <summary>"light", "dark" or "system" (the default): Glow's mode.</summary>
    AppearanceMode,
    /// <summary>The day mode's dot preset (an id; "indigo" by default).</summary>
    AppearanceDotsLight,
    /// <summary>The night mode's dot preset.</summary>
    AppearanceDotsDark,
    /// <summary>"preset" (the default) or "#rrggbb": your colour by day.</summary>
    AppearanceYouLight,
    /// <summary>Their colour by day.</summary>
    AppearanceThemLight,
    /// <summary>Your colour at night.</summary>
    AppearanceYouDark,
    /// <summary>Their colour at night.</summary>
    AppearanceThemDark,
    /// <summary>"on" (the default) or "off": the window's edge glows while something is live.</summary>
    AppearanceEdgeGlow,
    /// <summary>"system" (the default: Windows' Animation effects) or "still".</summary>
    AppearanceMotion,
    /// <summary>"on" (the default) or "off": local-only mode (Settings > AI's "Nothing leaves this computer").</summary>
    LlmLocalOnly,
    /// <summary>The typing speed the Stats screen measures time saved against: a whole number of words a minute, 10 to 200 (40 unless set).</summary>
    StatsTypingWpm,
    /// <summary>"on" (the default) or "off": a milestone reached, or a best set, is celebrated.</summary>
    StatsCelebrate,
    /// <summary>The weekdays the streak rests on: "none" (the default), or ISO weekdays ascending and comma-separated ("6,7"), never all seven.</summary>
    StatsRestDays,
    /// <summary>"shown" (the default) or "hidden": a hidden streak shows nowhere and celebrates nothing.</summary>
    StatsStreak,
    /// <summary>"on" or "off" (the default): the share card may carry the heatmap.</summary>
    StatsShareHeatmap,
    /// <summary>The first day (YYYY-MM-DD) of the week whose review the user dismissed.</summary>
    StatsReviewDismissed,
}

public static class ShellSettings
{
    /// <summary>The setting's key in the core's store.</summary>
    public static string Key(this ShellSetting setting) => setting switch
    {
        ShellSetting.OnboardingDone => "onboarding.done",
        ShellSetting.DictationPolish => "dictation.polish",
        ShellSetting.MeetingsCallsDefault => "meetings.calls.default",
        ShellSetting.MeetingsLlm => "meetings.llm",
        ShellSetting.AudioInput => "audio.input",
        ShellSetting.AudioOutput => "audio.output",
        ShellSetting.RetentionDays => "retention.days",
        ShellSetting.DictationKey => "dictation.key",
        ShellSetting.DictationEditKey => "dictation.edit_key",
        ShellSetting.DictationEnabled => "dictation.enabled",
        ShellSetting.ImportKeyNote => "import.key_note",
        ShellSetting.AppearanceMode => "appearance.mode",
        ShellSetting.AppearanceDotsLight => "appearance.dots.light",
        ShellSetting.AppearanceDotsDark => "appearance.dots.dark",
        ShellSetting.AppearanceYouLight => "appearance.you.light",
        ShellSetting.AppearanceThemLight => "appearance.them.light",
        ShellSetting.AppearanceYouDark => "appearance.you.dark",
        ShellSetting.AppearanceThemDark => "appearance.them.dark",
        ShellSetting.AppearanceEdgeGlow => "appearance.edge_glow",
        ShellSetting.AppearanceMotion => "appearance.motion",
        ShellSetting.LlmLocalOnly => "llm.local_only",
        ShellSetting.StatsTypingWpm => "stats.typing_wpm",
        ShellSetting.StatsCelebrate => "stats.celebrate",
        ShellSetting.StatsRestDays => "stats.rest_days",
        ShellSetting.StatsStreak => "stats.streak",
        ShellSetting.StatsShareHeatmap => "stats.share_heatmap",
        ShellSetting.StatsReviewDismissed => "stats.review_dismissed",
        _ => throw new ArgumentOutOfRangeException(nameof(setting)),
    };

    /// <summary>The id its setting.get and setting.set carry: "setting:" and the key.</summary>
    public static string CommandId(this ShellSetting setting) => $"setting:{setting.Key()}";
}

/// <summary>
/// Where the shell's diagnostics about commands go: a command's name and what kind of failure,
/// never its fields or anything the user said (a note's words travel in them).
/// </summary>
public sealed class ScreenLog(Action<string> write)
{
    public void Write(string message) => write(message);

    /// <summary>
    /// The debug trace, and <see cref="Also"/> once the app has set it (the local log). Built from
    /// command names and fixed words only.
    /// </summary>
    public static ScreenLog System { get; } = new(message =>
    {
        global::System.Diagnostics.Trace.WriteLine($"Inkwell screens: {message}");
        Also?.Invoke(message);
    });

    /// <summary>Where <see cref="System"/>'s lines also go: the local log, set once at launch (App).</summary>
    public static Action<string>? Also { get; set; }
}

/// <summary>An enum value's JSON name (its JsonStringEnumMemberName), through the source-generated serializer.</summary>
public static class Wire
{
    public static string Name<T>(T value) where T : struct, Enum
    {
        var json = JsonSerializer.Serialize(value, InkEventsJson.Default.GetTypeInfo(typeof(T))!);
        return JsonSerializer.Deserialize<string>(json, WireJson.Default.String)!;
    }
}

[System.Text.Json.Serialization.JsonSerializable(typeof(string))]
internal sealed partial class WireJson : System.Text.Json.Serialization.JsonSerializerContext;

/// <summary>A field written as JSON's null (a mode's polish_model: the AI setting's).</summary>
internal sealed class JsonNull
{
    public static JsonNull Value { get; } = new();

    private JsonNull() { }
}

/// <summary>Writes a command's fields as one JSON object: strings, numbers, booleans, null, lists and objects of them.</summary>
internal static class JsonFields
{
    public static string Write(IEnumerable<(string Key, object Value)> fields)
    {
        var sorted = new SortedDictionary<string, object>(StringComparer.Ordinal);
        foreach (var (key, value) in fields)
        {
            sorted[key] = value;
        }
        var buffer = new ArrayBufferWriter<byte>();
        using (var w = new Utf8JsonWriter(buffer))
        {
            Value(w, sorted);
        }
        return Encoding.UTF8.GetString(buffer.WrittenSpan);
    }

    private static void Value(Utf8JsonWriter w, object value)
    {
        switch (value)
        {
            case JsonNull:
                w.WriteNullValue();
                break;
            case string s:
                w.WriteStringValue(s);
                break;
            case bool b:
                w.WriteBooleanValue(b);
                break;
            case int i:
                w.WriteNumberValue(i);
                break;
            case long l:
                w.WriteNumberValue(l);
                break;
            case ulong u:
                w.WriteNumberValue(u);
                break;
            case SortedDictionary<string, object> obj:
                w.WriteStartObject();
                foreach (var (key, field) in obj)
                {
                    w.WritePropertyName(key);
                    Value(w, field);
                }
                w.WriteEndObject();
                break;
            case System.Collections.IEnumerable list:
                w.WriteStartArray();
                foreach (var item in list)
                {
                    Value(w, item!);
                }
                w.WriteEndArray();
                break;
            default:
                throw new ArgumentException($"a command field cannot be a {value.GetType().Name}");
        }
    }
}
