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
 *   (ink_register_engine). The ink's audio bands are copied out on demand (ink_bands_read).
 *   The event types are defined once, in schema/events.schema.json; the Swift types are generated
 *   from it (cargo run -p ink-ffi --bin ink-schema).
 *
 * STRINGS
 *   Every string crossing this ABI is NUL-terminated UTF-8. A string passed in is read during the
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
 *   5. Async engines answer through a completion call: the core hands every transcription a call
 *      id, and the engine answers it exactly once with ink_engine_complete, from any thread,
 *      either before its transcribe function returns (a synchronous engine) or later (an
 *      asynchronous one). The worker waits for that answer.
 *   6. The core never calls into the shell's main thread and never waits on it.
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

/* The version of this header. core.ready reports the core's; they must match. */
#define INK_ABI_VERSION 1

/* Status codes. Zero is success; every failure is negative. Where a call fails for a reason a
 * user could act on, an event says why as well. */
#define INK_OK 0
#define INK_ERR_NOT_INITIALIZED (-1)     /* no ink_init, or after ink_shutdown */
#define INK_ERR_ALREADY_INITIALIZED (-2) /* ink_init twice without ink_shutdown between */
#define INK_ERR_INVALID_ARGUMENT (-3)    /* a NULL pointer, bad UTF-8, JSON the call cannot read */
#define INK_ERR_FAILED (-4)              /* the call was understood and could not be done */
#define INK_ERR_UNKNOWN_CALL (-5)        /* ink_engine_complete for a call not waiting (see below) */
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
 *       Ends with "meeting.finished" (the record) or "meeting.failed".
 *   {"cmd":"model.warm","job":"dictation_final"}
 *       Loads the job's model and keeps it loaded. "model.warmed", "model.refused" or
 *       "model.warm_failed".
 *   {"cmd":"model.update","model":"<installed id>","next":"<registry id>"}
 *       Replaces a model's files. The model is held exclusively from before it is unloaded until
 *       the new one is installed and warm: meanwhile every job that needs it is refused with
 *       "model.refused", never served from files being replaced. "model.update_started", then
 *       "model.update_finished".
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

/* Engine kinds. Only offline ASR engines (dictation and meeting finals) are registered today;
 * live partials and language models get their own kinds when the Mac engines need them. */
#define INK_ENGINE_OFFLINE 1u

/*
 * An engine the shell owns (Apple accelerators stay in Swift, architecture rule 2).
 *
 * `info_json` (read during ink_register_engine, never kept):
 *   {"id":"<unique id>","licence":"<weights licence>",
 *    "jobs":[{"job":"dictation_final","wer":5.1},{"job":"meeting_final","wer":9.8}]}
 * Offline engines may fill "dictation_final" and "meeting_final". "wer" is the engine's measured
 * word error rate on that job: the router picks the lowest.
 *
 * transcribe (required): 16 kHz mono float samples, gain already applied, and
 *   `options_json` ({"channel":"mic"|"far","context":"<words to favour>"}; context optional).
 *   Both pointers are valid only until transcribe returns: copy the samples to answer later.
 *   Answer with ink_engine_complete(call, result) exactly once:
 *     {"segments":[{"start_ms":0,"end_ms":1200,"text":"..."}]}
 *     {"error":{"kind":"failed"|"cancelled"|"model_missing","message":"..."}}
 *   An error message must never contain what was said.
 * cancel (optional, may be NULL): the core no longer wants call `call`'s answer (the job was
 *   cancelled). Stop early if you can. Answer it anyway: the answer is then discarded.
 * release (optional, may be NULL): the core has let go of the engine (engine.unregister, or
 *   ink_shutdown). Called exactly once, after the last transcribe returned, on the thread that
 *   let go: the core's worker, or the thread calling ink_shutdown. It must not wait on the main
 *   thread. Nothing is called with `ctx` after it.
 */
typedef struct InkEngineVTable {
    uint32_t size; /* sizeof(InkEngineVTable): later versions append fields */
    uint32_t kind; /* INK_ENGINE_OFFLINE */
    const char *info_json;
    void *ctx;
    void (*transcribe)(void *ctx, uint64_t call, const float *samples, size_t len,
                       const char *options_json);
    void (*cancel)(void *ctx, uint64_t call);
    void (*release)(void *ctx);
} InkEngineVTable;

/*
 * Registers an engine; the table is copied. From here the router may pick it for its jobs, on
 * worker threads. Returns INK_OK ("engine.registered" follows), or INK_ERR_INVALID_ARGUMENT for
 * a malformed table or info, or INK_ERR_FAILED for an id already taken. On failure nothing is
 * kept and release is not called.
 */
int32_t ink_register_engine(const InkEngineVTable *vtable);

/*
 * Answers transcription `call` (see InkEngineVTable). Any thread. Returns INK_OK, or
 * INK_ERR_UNKNOWN_CALL when that call is not waiting: answered already, given up (cancelled or
 * shut down), or never issued. That is not an error the engine has to handle.
 */
int32_t ink_engine_complete(uint64_t call, const char *result_json);

/*
 * Stops the core (see SHUTDOWN). Returns INK_OK, or INK_ERR_NOT_INITIALIZED. ink_init may be
 * called again afterwards.
 */
int32_t ink_shutdown(void);

#ifdef __cplusplus
}
#endif

#endif /* INKWELL_H */
