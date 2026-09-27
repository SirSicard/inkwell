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
 *   The event types are defined once, in schema/events.schema.json; the Swift types are generated
 *   from it (cargo run -p ink-ffi --bin ink-schema).
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
 *   the user's words (dictation.inserted, meeting.partial, meeting.final): never log them.
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
 *    appended to InkEngineVTable for them, ink_stream_event, and the "unavailable" error kind. */
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
 *       "model.update_finished". While a job is using the model, or another update holds it,
 *       the update is "command.failed" and nothing changes: send it again later.
 *   {"cmd":"engine.unregister","engine":"<engine id>"}
 *       Lets go of an engine the shell registered; its release function runs once no call is in
 *       flight. "engine.unregistered".
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
 */
int32_t ink_bands_read(InkBands *out);

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
 *     {"id":"<unique id>","licence":"<weights licence>","model":"<model name>","local":true}
 *     "local" says whether the text stays on this machine; local-only mode refuses a model that
 *     says false. Registered language models do dictation polish.
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
