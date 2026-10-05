/*
 * inkwell.h: the C ABI of the Inkwell core.
 *
 * Hand-written, and the only header the native shells include. The Rust side is
 * core/crates/ink-ffi/src/lib.rs; the two are kept in step by hand, and tests/abi.rs drives every
 * function declared here from Rust the way a shell calls it.
 *
 * SHAPE
 *   Commands go in as JSON (ink_command). Events come out as JSON through one callback
 *   (ink_init). Engines the shell owns come in as a table of function pointers
 *   (ink_register_engine); their answers come back through ink_engine_complete, and a live
 *   stream's words through ink_stream_event. The ink's audio bands are copied out on demand
 *   (ink_bands_read).
 *   The event types are defined once, in schema/events.schema.json; the Swift and C# types are
 *   generated from it (cargo run -p ink-ffi --bin ink-schema).
 *
 * STRINGS
 *   Every string crossing this ABI is NUL-terminated UTF-8, and at most INK_MAX_JSON bytes. A string passed in is read during the
 *   call and never kept: the core copies what it needs. A string passed out is valid only for the
 *   duration of the callback that receives it: copy it to keep it.
 *
 * THREADS (the whole contract; ink-core/src/threading.rs names the threads)
 *   1. Event callbacks arrive on ONE core thread, "ink-events", in order. The callback must
 *      return promptly: hop to your main thread (DispatchQueue.main.async) and return. It must
 *      never wait on the main thread, and never call ink_shutdown. It may call ink_command.
 *   2. ink_init, ink_register_engine and ink_shutdown may be called from any thread except the
 *      event thread; ink_command from any thread, the event thread included. ink_init and
 *      ink_shutdown are serialised: one waits for the other. A call made while ink_shutdown runs
 *      returns INK_ERR_NOT_INITIALIZED.
 *   3. ink_bands_read may be called from any thread, at any rate, including a render callback: it
 *      never blocks, never allocates and never waits on the core.
 *   4. An engine's functions (InkEngineVTable) are called from core WORKER threads, never from the
 *      event thread and never from the thread that registered it. Several calls may be in flight
 *      at once, on different threads.
 *   5. Async engines answer through a completion call: the core hands every call it makes (a
 *      transcription, each step of a live stream, a generation) a call id, and the engine answers
 *      it exactly once with ink_engine_complete, from any thread, either before its function
 *      returns (a synchronous engine) or later (an asynchronous one). The worker waits for that
 *      answer.
 *   6. A live stream's events (ink_stream_event) may be sent from any thread, one at a time per
 *      stream, in the order they happened. ink_stream_event only queues: it never waits on the
 *      core's workers, so it may be called from inside the engine's own functions.
 *   7. The core never calls into the shell's main thread and never waits on it.
 *
 * LOGGING
 *   The core installs the only logger for the core's code (a `log` logger and a `tracing`
 *   subscriber) in ink_init. It drops what may carry a secret or the user's words: HTTP client
 *   debug output (which prints API-key headers) and llama.cpp's debug output (which quotes
 *   generated text). Shells must not install their own logger or subscriber for the core's
 *   targets; there is nothing to configure beyond "log_level" in the config. Event payloads carry
 *   the user's words (dictation.inserted, dictation.partial, meeting.partial, meeting.final, and
 *   the library's answers: library.records, library.search, library.record, commitments.listed):
 *   never log them.
 *
 * SHUTDOWN
 *   ink_shutdown stops every worker, lets go of every engine the shell registered (their release
 *   functions run before it returns) and unloads every model, then stops the event thread after
 *   delivering "core.stopped". When it returns, no callback of any kind arrives any more.
 *   Call it on every quit path the shell controls: llama.cpp's Metal backend aborts the process
 *   at exit if a model is still loaded.
 */

#ifndef INKWELL_H
#define INKWELL_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The version of this header. core.ready reports the core's; they must match.
 * 2: live-stream and language-model engines (INK_ENGINE_STREAMING, INK_ENGINE_LLM), the fields
 *    appended to InkEngineVTable for them, ink_stream_event, and the "unavailable" error kind.
 *    Added since without a new version (every addition is a new name, nothing changed shape):
 *    ink_far_bands_read, the meetings commands, and a language model's optional
 *    "context_tokens". */
#define INK_ABI_VERSION 2

/* The longest JSON string the core reads (config, command, engine info, engine answer), in bytes
 * without the NUL. A longer one is refused unread with INK_ERR_INVALID_ARGUMENT; an engine answer
 * that long still answers its call, as a failure. */
#define INK_MAX_JSON (1u << 20)

/* Status codes. Zero is success; every failure is negative. Where a call fails for a reason a
 * user could act on, an event says why as well. */
#define INK_OK 0
#define INK_ERR_NOT_INITIALIZED (-1)     /* no ink_init, or after ink_shutdown */
#define INK_ERR_ALREADY_INITIALIZED (-2) /* ink_init twice without ink_shutdown between */
#define INK_ERR_INVALID_ARGUMENT (-3)    /* a NULL pointer, bad UTF-8, JSON the call cannot read */
#define INK_ERR_FAILED (-4)              /* the call was understood and could not be done */
#define INK_ERR_UNKNOWN_CALL (-5)        /* a call not waiting, or a stream not open (see below) */
#define INK_ERR_PANIC (-6)               /* a bug in the core; it was contained at the boundary */

/*
 * An event. `event_json` is one JSON object whose "type" names the event (schema/events.schema.json)
 * and `len` is its length in bytes, without the terminating NUL. Both are valid only during the
 * call. `ctx` is what ink_init was given. Runs on the event thread (see THREADS 1).
 */
typedef void (*InkEventCallback)(void *ctx, const char *event_json, size_t len);

/*
 * Starts the core. `config_json`:
 *   {
 *     "data_dir":   "<absolute path>",  required: the library, recordings and models live here
 *     "models_dir": "<absolute path>",  optional: models elsewhere (default <data_dir>/models)
 *     "log_level":  "info",             optional: off | error | warn | info | debug | trace
 *     "log_stderr": true                optional: write log lines to stderr (default true)
 *   }
 * `cb` receives every event; `ctx` is passed back to it and must stay valid until ink_shutdown
 * returns. The first event is "core.ready". Returns INK_OK, or an error with nothing started.
 */
int32_t ink_init(const char *config_json, InkEventCallback cb, void *ctx);

/*
 * Queues a command. `command_json` is one object with a "cmd" field; an optional "id" string is
 * echoed in "command.failed" so the shell can match a failure to its command. Commands run one
 * at a time, in the order they were queued.
 *
 *   {"cmd":"replay_meeting","mic":"<wav>","far":"<wav>","title":"...","pacing":"fast"}
 *       A meeting whose capture is WAV files instead of devices: the same pump, chunks on disk,
 *       live chain and final pass as a real meeting (architecture rule 7). "far" and "title" are
 *       optional. "pacing" is "realtime" (default) or "fast"; "fast" needs files of at most 55 s.
 *       Ends with "meeting.finished" (the record) or "meeting.failed". A file that cannot be
 *       read, or a meeting already running, is "command.failed" and starts nothing.
 *   {"cmd":"model.warm","job":"dictation_final"}
 *       Loads the job's model and keeps it loaded. "model.warmed", "model.refused" or
 *       "model.warm_failed".
 *   {"cmd":"model.update","model":"<installed id>","next":"<registry id>"}
 *       Replaces a model's files. The model is held exclusively from before it is unloaded until
 *       the new one is installed and warm: meanwhile every job that needs it is refused with
 *       "model.refused", never served from files being replaced. "model.update_started", then
 *       "model.update_progress" as the download goes (about four a second, and once when every
 *       byte is on disk), then "model.update_finished". While a job is using the model, or
 *       another update holds it, the update is "command.failed" and nothing changes: send it
 *       again later.
 *       With "model" and "next" the same registry id, it installs that model: meant for a model
 *       that is not installed yet (its first download), sent only when the user asks for it.
 *       Nothing else is unloaded or warmed. An installed model keeps its files, but if it is
 *       loaded it is unloaded (and loaded again if it was warm), and while a job uses it the
 *       update is refused, as above. A voice detector installed while dictation runs is taken by
 *       it at once ("dictation.voice_detection" says so).
 *   {"cmd":"engine.unregister","engine":"<engine id>"}
 *       Lets go of an engine the shell registered; its release function runs once no call is in
 *       flight. "engine.unregistered".
 *
 *   Meetings run on their own thread (starting one never waits behind a model download), and
 *   questions about one on another. A failure is "command.failed" with the "id".
 *   {"cmd":"meeting.start","app":"<app id>","title":"..."}
 *       Records a meeting from this machine: the mic (with Bluetooth output, the built-in one
 *       unless "meetings.headset_mic" is on) and the far end ("app", when the start answers a
 *       "meeting.detected" offer: that app; otherwise everything this machine plays). Both
 *       optional. "meeting.started" (with the title, the app's name, the mic and
 *       "delete_until_unix_ms"), then the live events. Only when the user asks, or for an app the
 *       user chose Always for (below), which starts here the same way, with "auto" in
 *       "meeting.started": the shell shows every recording from that event.
 *   {"cmd":"meeting.stop"}
 *       Ends the recording; the final pass follows ("meeting.stopped" ... "meeting.finished").
 *       A meeting started for an app also ends by itself 15 s after that app lets go of the
 *       microphone.
 *   {"cmd":"meeting.discard"}
 *       "Stop and delete", within a minute of a start made here (until "delete_until_unix_ms"):
 *       the recording ends, no final pass runs, and the record and its audio are deleted as if
 *       never made ("meeting.stopped", then "meeting.discarded"). Later it is refused with code
 *       "delete_window_over": stop it, then delete it from the library. Its answer is what
 *       happens: refused when the meeting had already stopped and is being finished, or had
 *       failed, so no "meeting.discarded" is waited for that cannot come.
 *   {"cmd":"meeting.dismiss","app":"<app id>"}
 *       "Not this one": the offer ends ("meeting.detection_ended" with "dismissed") and that app
 *       is not offered again until it releases the microphone.
 *   {"cmd":"meeting.ask","question":"...","id":"<ref>"}
 *       A question about the live meeting, answered from its transcript so far by the language
 *       model the shell registered: "meeting.answered" with the "id" as "ref". The answer is the
 *       model's words: render them as text only.
 *   {"cmd":"meetings.recover"}
 *       Finishes the meetings a crash interrupted (their audio repaired, their final pass run):
 *       "meeting.recovered" and the pass's events per meeting, then "meetings.recovered". Send it
 *       once the shell's own engines are registered, so a recovered meeting gets them too. Sent
 *       while a recovery runs, it is never refused: that recovery goes one more round (asks
 *       during a round count as one), ending with its own "meetings.recovered".
 *   {"cmd":"meetings.calls.list","id":"<ref>"}
 *       "meetings.calls": the default call policy and every app seen holding the microphone for a
 *       call, or chosen for (at most 64), with its policy and whether it was chosen.
 *   {"cmd":"meetings.calls.set","app":"<app id>","policy":"always|ask|never|default","id":"<ref>"}
 *       One app's call policy, by the identity detection reports (never its name): always (its
 *       calls are recorded at once, visibly), ask (offered), never (neither), or default (follow
 *       the default again). Saved and applied at once: an offered app set to never is withdrawn
 *       ("meeting.detection_ended", dismissed), a held app set to ask is offered. An app offered
 *       and set to always stays offered: send meeting.start for it ("Always for this app").
 *       Answers "meetings.calls" with the "id" as "ref". While the stored choices cannot be read
 *       ("meetings.calls" has a "message"), it is refused with code "list_unreadable" unless it
 *       says "replace_unreadable":true, which starts the list over with this choice (and, under
 *       a default of always, sets the default to ask, said in the answer's "message").
 *   Detection listens while any app could be offered or recorded (the default call policy,
 *   "meetings.calls.default", is not never, or an app is chosen always or ask): "meeting.detection"
 *   says whether it listens. An app that has held the microphone for 3 s is offered
 *   ("meeting.detected"; "meeting.detection_ended" takes the offer back) when its policy is ask,
 *   recorded when it is always (offered instead, with a "message", when that start fails or its
 *   own sound cannot be recorded alone, so the recording would hold everything this computer
 *   plays; and offered after the user stopped a recording by hand during this call), and left
 *   alone when it is never. On Windows an app's identity is compared and kept in lowercase,
 *   in every event and command. Unasked "meetings.calls" says the list changed (a new app seen,
 *   the default set).
 *
 *   The screens' commands run on their own thread, in order among themselves, so a model update
 *   holding the commands above never delays them. Each answers with the event named, or
 *   "command.failed" (with the "id").
 *   {"cmd":"permissions.check"}
 *       "permissions.checked": microphone, system audio, accessibility, input monitoring. Never
 *       prompts. System audio takes about a second once the app has asked for it, and reads
 *       not_determined until then. Check when the user comes back from System Settings, not on a
 *       timer.
 *   {"cmd":"permission.request","permission":"microphone|system_audio|accessibility|input_monitoring"}
 *       Shows the system prompt the first time, else opens the settings pane: only when the user
 *       asks. "permission.requested"; the answer comes later, so check again afterwards.
 *   {"cmd":"commitments.list","limit":200}
 *       "commitments.listed": the open commitments, soonest due first ("limit" optional, 1-1000).
 *   {"cmd":"commitment.set_done","commitment":"<id>","done":true}
 *       "commitment.updated". Settles a looks-done suggestion either way.
 *   {"cmd":"commitment.not_yet","commitment":"<id>"}
 *       Dismisses a looks-done suggestion; the commitment stays open. "commitment.updated".
 *   {"cmd":"note.add","record":"<record id>","at_ms":754000,"text":"...","id":"<ref>"}
 *   {"cmd":"note.update","note":"<note id>","text":"..."}
 *   {"cmd":"note.delete","note":"<note id>"}
 *       A record's notes: "note.added" (with the note's id), "note.updated", "note.deleted",
 *       each with the command's "id" as "ref"; a failure is "command.failed" with that "id". So
 *       every answer can be matched to the line that sent it. The note's words are never echoed
 *       back.
 *   {"cmd":"speaker.name","record":"<record id>","speaker":"spk1","name":"...","id":"<ref>"}
 *       Names (or renames) a far-end speaker of a record: "speaker" is the diarizer's label, as
 *       the record's segments carry it, and only a label its far end has (the mic is the user,
 *       never renamed). The name is trimmed; an empty one clears it, and the speaker reads as
 *       numbered again. One line, at most 80 characters. "speaker.named" (with the "id" as "ref",
 *       and "named": false once cleared), or "command.failed"; the name is never echoed back.
 *       record.open's "speakers" carries it, and Ask's transcript names that speaker by it.
 *   {"cmd":"record.delete","record":"<record id>","id":"<ref>"}
 *       Deletes one record whole, of any kind, as the retention setting deletes one: its
 *       transcript, notes, summary, commitments, speaker names and search entries, its words
 *       overwritten in the library's files, then its audio. "record.deleted" (with the "id" as
 *       "ref"; "audio_left" when its audio stayed on disk, "scrubbed": false while another
 *       process keeps the words in the database's log), or "command.failed": a record that is
 *       not there, or one still live (a meeting being recorded, or whose final pass has not
 *       finished). Only when the user asks, after a confirmation that it cannot be undone.
 *   {"cmd":"models.list"}
 *       "models.listed": the catalogue's models for this OS, their measured error rates and
 *       whether each is installed. Send engine.route for what serves a job now. A model the
 *       shell runs fills no job there: the core only downloads it (model.update) into
 *       <models_dir>/<id>/<first 12 digits of its revision>/, and the shell loads it from there
 *       and registers its engine. The Mac's Parakeet, parakeet-tdt-0.6b-v3-coreml: its files are
 *       in that directory's parakeet-tdt-0.6b-v3/, the folder FluidAudio loads v3 from.
 *   {"cmd":"engine.route","job":"dictation_final"}
 *       Which engine serves a job now: "engine.routed" with the job, and the engine's id and
 *       source ("registry" for a downloaded model, "shell" for an engine the shell registered),
 *       or no id when nothing fills it. The router picks the lowest measured error rate among
 *       installed models and registered engines, at every call: a shell engine registered as a
 *       fallback serves until a better model finishes installing, then that model does. A
 *       download never delays the answer, which can overtake an engine.unregister or model.update
 *       sent before it: ask again once their event has come.
 *   {"cmd":"setting.get","key":"<key>"}
 *   {"cmd":"setting.set","key":"<key>","value":"<value>"}
 *       "setting.value". Only the shell's settings: "onboarding.done" (true|false),
 *       "dictation.polish" (on|off; setting.set takes only off, which also withdraws polish's
 *       consent in the same write, and answers "consent.state" too: consent.allow turns it on),
 *       "dictation.key" (any key this computer can watch, as hotkey.check judges it: a
 *       right-hand modifier, or fn on macOS, held on its own; a function key; or modifiers and
 *       one key such as ctrl+shift+space. Stored in hotkey.check's one spelling, which
 *       "setting.value" echoes; a key it refuses is refused here with its reason. The named
 *       tokens fn|right_option|right_command|right_control|right_shift|right_alt|right_win are
 *       stored on either OS: one this OS cannot hold is refused when dictation binds it, as
 *       "dictation.off" with "key_refused". The default is fn on macOS and right_control on
 *       Windows),
 *       "dictation.edit_key" (off or a key as for dictation.key, never the dictation key in any
 *       spelling; voice edit turns on with its consent through consent.allow, and off withdraws
 *       that consent in the same write, answering "consent.state" too; an edit key set without a
 *       consent edits nothing),
 *       "dictation.enabled" (on|off: the shell's own switch, read before it sends
 *       dictation.enable), "meetings.calls.default" (ask|always|never: the call policy for apps
 *       not chosen for; ask unless set), "meetings.detect" (on|off: the old "Offer to record
 *       calls", answered for the default: off is never, on over never is ask), "meetings.headset_mic" (on|off),
 *       "meetings.llm" (on|off: a meeting's summary and Ask; as for dictation.polish, setting.set
 *       takes only off, which also withdraws their consent, and consent.allow turns it on),
 *       "llm.local_only" (on|off: on unless turned off, and on when unreadable; while on, a
 *       language model whose info says "local":false is never called, for polish, voice edit,
 *       summaries or Ask, and the call fails saying so) and "retention.days"
 *       (forever|7|30|90|365: meetings and dictations older than that are deleted, never
 *       imports; at launch, after each meeting and when it changes, on the core's own thread;
 *       "library.swept" says how many), "import.key_note" (dismissed: import.notes stops
 *       saying what became of 0.2's hotkey), "stats.typing_wpm" (a whole number from 10 to 200,
 *       written plainly: the typing speed stats.get measures time saved against; 40 unless set)
 *       and "stats.celebrate" (on|off: milestones are celebrated; on unless set). A change to the keys or to dictation.polish reaches a
 *       running dictation at once (keys rebound): a new "dictation.ready" (or "dictation.off")
 *       follows the "setting.value".
 *   {"cmd":"hotkey.check","binding":"<token>","id":"<ref>"}
 *       Whether this computer can watch a key binding as the dictation or edit key, before the
 *       shell stores one the user recorded: "hotkey.checked" with "ok", and either "canonical"
 *       (its one spelling, to store and to compare keys by: modifiers on macOS in the order
 *       fn ctrl option shift cmd, e.g. ctrl+shift+space) or "reason" (why not, in plain words to
 *       show after "can't use that:"). Nothing is stored. The platform's own parser answers, so a
 *       key binds exactly when this says yes.
 *   {"cmd":"dictation.enable","utc_offset_minutes":120,"id":"<ref>"}
 *       Dictation live: the core holds the keys (the dictation key, and the edit key if one is
 *       set), opens the mic at the first press and lets it go after 1 minute without a take.
 *       Sent again while live, it reads the settings and binds the keys again (after the user
 *       granted Accessibility, say). "dictation.ready" names the keys held; "dictation.off" says
 *       why dictation is not live (needs_accessibility, key_refused, unsupported, ...). Both carry
 *       the command's "id" as "ref". "utc_offset_minutes" (optional) is for {date} and {time}.
 *   {"cmd":"dictation.disable","id":"<ref>"}
 *       Lets go of the keys and the mic: "dictation.off" with reason disabled.
 *   {"cmd":"consent.get","feature":"polish|edit|meetings","id":"<ref>"}
 *       "consent.state" for a feature that sends the user's words to a language model (polish:
 *       the dictation; edit: the selection and the instruction; meetings: a meeting's transcript,
 *       for its summary, its commitments and Ask): its switch, where the model it
 *       would use now sends them ("to": on_device or cloud, with its "name", and for cloud the
 *       "endpoint"), and where the user agreed it may ("allowed_to"); "allowed" says whether
 *       that consent covers the model now. Each consent is its own. A feature runs only when on
 *       and allowed: a model that changed destination since the user agreed gets nothing, and
 *       each take says so (dictation.warning polish_not_allowed, dictation.edit_failed
 *       not_allowed; a meeting finishes with meeting.warning summary_not_allowed and no summary,
 *       and meeting.ask fails asking for the user's OK). Send it again after an engine.registered or engine.unregistered of a
 *       language model.
 *   {"cmd":"consent.allow","feature":"polish|edit|meetings","to":"on_device|cloud","endpoint":"<for cloud>",
 *    "key":"<for edit: its key>","id":"<ref>"}
 *       The user agreed, after the shell told them plainly where the feature sends their words:
 *       records that consent and turns the feature on (its switch, or edit's key), both in
 *       one write. Name what "consent.state" showed; if the model changed meanwhile, nothing is
 *       recorded and it fails ("command.failed", with a fresh "consent.state" first), so the
 *       shell asks again. Answers "consent.state" with the "id".
 *   {"cmd":"llm.providers","id":"<ref>"}
 *   {"cmd":"llm.key.save","provider":"openai|groq|anthropic|openrouter|custom","key":"...","id":"<ref>"}
 *   {"cmd":"llm.key.delete","provider":"<provider>","id":"<ref>"}
 *   {"cmd":"llm.choose","provider":"<provider>|none","model":"<optional>","base_url":"<custom only>",
 *    "local_only":"off","id":"<ref>"}
 *       Own-key language models, for a shell with no model of its own (Windows): "llm.providers"
 *       lists every provider, whether its key is stored (asked without reading it) and the one
 *       chosen. A key goes only into the OS key store (macOS keychain, Windows Credential
 *       Manager): never into settings, an event, an error or a log; send it once and forget it.
 *       llm.choose picks the provider and its model: one that is not on this machine is chosen
 *       only with "local_only":"off", which turns local-only mode off with it; one on this
 *       machine, or none, turns it back on. It answers "setting.value" (llm.local_only) and a
 *       "consent.state" per feature first: choosing sends nothing, and each feature still needs
 *       its consent for the provider's endpoint. A model the shell registered is used before the
 *       chosen provider. Each answers "llm.providers" with the "id".
 *   {"cmd":"llm.test","id":"<ref>"}
 *       One short fixed request (never the user's words) to the chosen provider with its stored
 *       key, through local-only mode: "llm.tested" with the "id", saying whether it answered
 *       (and the HTTP status of a refusal). One at a time; another sent meanwhile fails as busy.
 *       A provider that has not answered within 60 s fails it, and ink_shutdown never waits for
 *       its answer.
 *   {"cmd":"modes.list"}
 *       "modes.listed": the user's modes, in the order they are matched, with the app identities
 *       each is picked for (on macOS, bundle ids: name them, never show them as they are).
 *   {"cmd":"snippets.list","id":"<ref>"}
 *   {"cmd":"snippets.save","snippets":[{"id":"...","trigger":"...","expansion":"...",
 *    "category":"...","enabled":true}],"id":"<ref>"}
 *       "snippets.listed": the snippets dictation expands, in order ("from_import" while they are
 *       still the Inkwell 0.2 import's). A save replaces the whole list and reaches a running
 *       dictation at once. Ids must be present and unique. A save over a stored list the core
 *       cannot read is refused (command.failed, "code":"list_unreadable") unless it says
 *       "replace_unreadable":true: the user chose to start over. At most 2000 items; an id, trigger, category or wake word at
 *       most 256 characters, an expansion or a command's value at most 16384.
 *   {"cmd":"voice_commands.list","id":"<ref>"}
 *   {"cmd":"voice_commands.save","enabled":false,"wake_prefix":"inkwell","commands":[{"id":"...",
 *    "triggers":["..."],"action":"insert_text","value":"...","enabled":true}],"id":"<ref>"}
 *       "voice_commands.listed": the switch, the wake word and the commands, each with
 *       "carried_out" (the core does change_style, toggle_polish and insert_text; the rest are
 *       recognised but not done in this build). A save replaces them all, at once, and takes
 *       "replace_unreadable" as snippets.save does.
 *   {"cmd":"import.notes","id":"<ref>"}
 *       "import.notes": what became of Inkwell 0.2's dictation hotkey ("key"), while there is
 *       something to say and until setting.set import.key_note dismissed.
 *   {"cmd":"import.check","id":"<ref>"}
 *       "import.checked": Inkwell 0.2's data at 0.2's own data directory on this computer (the
 *       core knows where; a shell never names it). "state" is found, with the dry run's
 *       "counts"; absent; imported (this library holds an import already, and 0.2's data is not
 *       opened); or unreadable, with "message" in words to show (0.2 in the middle of a save,
 *       say). 0.2's data is only read, and the keychain is not asked ("linked_keys" is 0).
 *   {"cmd":"import.run","id":"<ref>"}
 *       Imports it in one transaction: dictations, dictionary, snippets, modes, 0.2's settings,
 *       voice commands and app styles; API keys already in the keychain are linked, asked for by
 *       existence only. "import.finished" with the "counts" written; a failure (no data, already
 *       imported, 0.2 in the middle of a save, a file 0.2 could not have written) is
 *       "command.failed", its message in words to show. A running dictation uses what came
 *       over at once; list the library again, and ask import.notes.
 *       These seven answer with the command's "id" as "ref"; a failure is "command.failed".
 *   {"cmd":"records.list","kind":"meeting","limit":50,"before":{"started_at_unix_ms":0,"id":"..."}}
 *       "library.records": records newest first (by start time, then id). All fields optional:
 *       "kind" is meeting, dictation or file_import; "limit" 1-500 (default 50); "before" is the
 *       last record of the previous page. "more" says whether another page follows.
 *   {"cmd":"records.search","query":"<words>","limit":50}
 *       "library.search": full-text matches across every record's current transcript, best first.
 *   {"cmd":"record.open","record":"<record id>"}
 *       "library.record": the record whole: transcript, notes, summary (markdown, to be rendered),
 *       commitments, named speakers, and its audio chunks placed on its timeline ("estimated"
 *       when the meeting's start was not written; "left_out" counts chunks that cannot be
 *       played or placed). An audio directory outside the library is refused: command.failed.
 *   {"cmd":"library.stats","since_unix_ms":0}
 *       "library.stats": per kind, records, time and words since the moment (at most 31 days
 *       back: it reads each transcript to count words; an older moment is refused); and how many
 *       of the newest meetings in a row kept the user's words and none of the far end's.
 *       These four answer with the command's "id" as "ref"; a failure is "command.failed" with
 *       that "id", so a screen can tell "could not load" from "empty".
 *   {"cmd":"stats.get","utc_offsets":[{"from_unix_ms":0,"minutes":60}],"week_start":1,
 *    "id":"<ref>"}
 *       "stats.counted": the Stats screen's numbers, counted on this computer from the library:
 *       words dictated, speed against the user's own past, time saved against stats.typing_wpm,
 *       the streak and a heatmap of words per day; meetings' hours, talk time (mic is the user,
 *       far end the others), longest monologue and the user's lines ending in a question mark
 *       (?, ？ or ؟); promises kept, open and overdue; and which milestones are reached. Days are
 *       the user's: "utc_offsets" is the zone's UTC offset over time, oldest first, each from the
 *       moment it took effect (the first also covers everything before it; 1 to 400 of them,
 *       minutes -840 to 840), and "week_start" the ISO weekday weeks start on (1 Monday to 7
 *       Sunday). It answers with the "id" as "ref"; a failure is "command.failed" with that "id".
 *   {"cmd":"milestones.check","utc_offsets":[...],"week_start":1,"id":"<ref>"}
 *       "milestones.reached": the milestones (words dictated 1,000 to 100,000, streaks of 7 to
 *       100 days) reached since the last check, to celebrate; usually none. Each is reported
 *       once ever, remembered in the library. A library's first check reports none and notes
 *       what is already reached; with stats.celebrate off a milestone is noted, never reported.
 *       Send it at launch and after a dictation or a meeting ends. Takes stats.get's calendar.
 *
 * Returns INK_OK once the command is queued; its outcome arrives as events. A command the core
 * cannot read returns INK_ERR_INVALID_ARGUMENT and queues nothing.
 */
int32_t ink_command(const char *command_json);

/* The latest bands of the live audio, for the ink: RMS amplitude per band, linear full scale. */
typedef struct InkBands {
    float low;          /* 80-500 Hz */
    float mid;          /* 500 Hz-2 kHz */
    float high;         /* 2-8 kHz */
    uint64_t published; /* bands published so far; unchanged since the last read = nothing new */
} InkBands;

/*
 * Copies the latest bands into `*out`. Never hands out a pointer into the core. Before ink_init,
 * after ink_shutdown, or while nothing is live, it copies zeros with the last count (see THREADS
 * 3 for where it may run). Returns INK_OK, or INK_ERR_INVALID_ARGUMENT for a NULL `out`.
 * ink_bands_read reads your mic's; ink_far_bands_read the far end's, during a meeting.
 */
int32_t ink_bands_read(InkBands *out);
int32_t ink_far_bands_read(InkBands *out);

/* Engine kinds. An engine is one kind; its table fills that kind's functions and leaves the
 * others NULL. */
#define INK_ENGINE_OFFLINE 1u   /* transcribes a whole buffer: dictation and meeting finals */
#define INK_ENGINE_STREAMING 2u /* live partials: a stream per meeting side (ABI 2) */
#define INK_ENGINE_LLM 3u       /* a language model: dictation polish (ABI 2) */

/*
 * An engine the shell owns (Apple accelerators stay in Swift, architecture rule 2).
 *
 * `info_json` (read during ink_register_engine, never kept):
 *   offline and streaming engines:
 *     {"id":"<unique id>","licence":"<weights licence>",
 *      "jobs":[{"job":"dictation_final","wer":5.1},{"job":"meeting_final","wer":9.8}]}
 *     Offline engines may fill "dictation_final" and "meeting_final"; streaming engines
 *     "live_partials". "wer" is the engine's measured word error rate on that job: the router
 *     picks the lowest.
 *   language models:
 *     {"id":"<unique id>","licence":"<weights licence>","model":"<model name>","local":true,
 *      "context_tokens":4096}
 *     "local" says whether the text stays on this machine; local-only mode refuses a model that
 *     says false. "context_tokens" (optional, at least 256) is how many tokens its context holds,
 *     prompt and answer together: a meeting's summary and Ask are sized to fit it (4096 when not
 *     said). Registered language models do dictation polish, meeting summaries, commitments and
 *     Ask.
 * Ids are unique across every kind.
 *
 * ANSWERS. Every call below that takes a `call` id is answered with ink_engine_complete(call,
 * result) exactly once, now or later, from any thread:
 *     {"segments":[{"start_ms":0,"end_ms":1200,"text":"..."}]}   transcribe
 *     {"ok":true}                                                 stream_open, _push, _finish
 *     {"text":"..."}                                              generate
 *     {"error":{"kind":"failed","code":42}}                       any of them
 *   "kind" is "failed", "cancelled", "model_missing", "unavailable" (the engine cannot run on this
 *   Mac now: a system feature is off, unsupported or not ready), or "bad_request" (the engine
 *   could not read the samples, options or request). "code" is optional: an integer of the
 *   engine's own, shown in the core's error. No text of the engine's is read: an error never
 *   carries free text into an event or a log, because an engine's words could quote what it
 *   heard. An engine that cannot answer truthfully answers an error: never a made-up result.
 *
 * INK_ENGINE_OFFLINE
 * transcribe (required): 16 kHz mono float samples, gain already applied, and
 *   `options_json` ({"channel":"mic"|"far","context":"<words to favour>"}; context optional).
 *   Both pointers are valid only until transcribe returns: copy the samples to answer later.
 *
 * INK_ENGINE_STREAMING (the four functions are required; the table must be this header's size)
 *   A stream's life is stream_open, stream_push any number of times, stream_finish (unless the
 *   core abandons the stream), then stream_close. The core numbers streams; `stream` is that
 *   number. Calls for one stream come from one worker thread at a time, in that order; different
 *   streams may run at once on different threads.
 * stream_open: a stream for one side, `options_json` {"channel":"mic"|"far"} (valid only during
 *   the call). Answer {"ok":true} once events may be sent for it, or an error.
 * stream_push: the stream's next 16 kHz mono samples, gain applied, valid only during the call:
 *   copy them. Answer promptly, well before the audio would have finished playing (the core
 *   gives up after 2 s and closes the stream): do the recognition elsewhere. An error answer
 *   ends the stream.
 * stream_finish: no more audio. Recognise what is left, send every trailing event, then answer.
 *   (The core gives up after 30 s.)
 * stream_close: the core is done with `stream`, after its finish was answered or instead of it
 *   (an error, a timeout, shutdown). No answer. Called exactly once for every stream_open,
 *   whatever it answered. Events sent for the stream after it are refused.
 * Events go to ink_stream_event(stream, event_json) as they happen (see THREADS 6):
 *     {"partial":"<text>"}  the not-yet-settled words; each partial replaces the last, and an
 *                           empty one clears it. Partials are never stored.
 *     {"final":{"start_ms":0,"end_ms":1200,"text":"..."}}  settled words, in ms from the stream's
 *                           first sample. Each final carries only its own words, never the
 *                           stream's text so far.
 *     {"stalled":{"code":1}}  the engine has fallen behind real time ("code" optional).
 *
 * INK_ENGINE_LLM (generate is required; the table must be this header's size)
 * generate: `request_json` (valid only during the call; it holds the user's words, never log it):
 *     {"system":"...","user":"...","max_tokens":1024,"temperature":0.3,"json_schema":"..."}
 *   "json_schema" is present only for structured answers. Answer {"text":"..."} or an error. The
 *   core gives up after 120 s.
 *
 * cancel (optional, any kind; may be NULL): the core no longer wants call `call`'s answer (the
 *   job was cancelled, the core gave up waiting, or it is shutting down). Stop early if you can.
 *   Answer it anyway: the answer is then discarded.
 * release (optional, may be NULL): the core has let go of the engine (engine.unregister, or
 *   ink_shutdown). Called exactly once, after every call returned and every stream was closed,
 *   on the thread that let go: the core's worker, or the thread calling ink_shutdown. It must not
 *   wait on the main thread. Nothing is called with `ctx` after it.
 */
typedef struct InkEngineVTable {
    uint32_t size; /* sizeof(InkEngineVTable): later versions append fields */
    uint32_t kind; /* INK_ENGINE_* */
    const char *info_json;
    void *ctx;
    void (*transcribe)(void *ctx, uint64_t call, const float *samples, size_t len,
                       const char *options_json);
    void (*cancel)(void *ctx, uint64_t call);
    void (*release)(void *ctx);
    /* Appended in ABI 2. A table the size of ABI 1's (ending at release) still registers an
     * offline engine. */
    void (*stream_open)(void *ctx, uint64_t call, uint64_t stream, const char *options_json);
    void (*stream_push)(void *ctx, uint64_t call, uint64_t stream, const float *samples,
                        size_t len);
    void (*stream_finish)(void *ctx, uint64_t call, uint64_t stream);
    void (*stream_close)(void *ctx, uint64_t stream);
    void (*generate)(void *ctx, uint64_t call, const char *request_json);
} InkEngineVTable;

/*
 * Registers an engine; the table is copied. From here the router may pick it for its jobs, on
 * worker threads. Returns INK_OK ("engine.registered" follows), or INK_ERR_INVALID_ARGUMENT for
 * a malformed table or info, or INK_ERR_FAILED for an id already taken. On failure nothing is
 * kept and release is not called.
 */
int32_t ink_register_engine(const InkEngineVTable *vtable);

/*
 * Answers `call` (see ANSWERS above). Any thread. Returns INK_OK, or INK_ERR_UNKNOWN_CALL when
 * that call is not waiting: answered already, given up (cancelled, timed out or shut down), or
 * never issued. That is not an error the engine has to handle. An answer the core cannot read
 * still answers the call, as a failure, and returns INK_ERR_INVALID_ARGUMENT.
 */
int32_t ink_engine_complete(uint64_t call, const char *result_json);

/*
 * An event of live stream `stream` (see INK_ENGINE_STREAMING). Any thread (THREADS 6); it only
 * queues. Returns INK_OK, INK_ERR_UNKNOWN_CALL when the stream is not open (never opened, or
 * closed: a late event is dropped), or INK_ERR_INVALID_ARGUMENT for an event it cannot read.
 */
int32_t ink_stream_event(uint64_t stream, const char *event_json);

/*
 * Stops the core (see SHUTDOWN). Returns INK_OK, or INK_ERR_NOT_INITIALIZED. ink_init may be
 * called again afterwards.
 */
int32_t ink_shutdown(void);

#ifdef __cplusplus
}
#endif

#endif /* INKWELL_H */
