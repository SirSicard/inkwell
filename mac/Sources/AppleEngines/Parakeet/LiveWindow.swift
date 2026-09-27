// Live partials by re-decoding a trailing window: the decisions, with no model and no threads.
//
// The stream keeps the audio of the utterance not yet settled (plus a little silence in front of
// it). Every `hop` of new audio it decodes all of that again with the offline model, which sees
// the whole utterance each time: the words shown as a partial are the offline model's words for
// what has been said so far. An utterance is settled (a final) when
//   - a pause follows its last word: the decoded words end at least `pause` before the window's
//     end, or
//   - it has grown to `maxUtterance` of audio: the words that end before the last `tailGuard` are
//     settled at a word boundary, and the rest stay pending.
// Both are measured in samples of audio, never with a timer.
//
// A partial leaves out the words that end in the newest `hideNewest` (0.16 s) of the decoded
// audio: those are the words the next decode most often changes. On the AMI replay this cut the
// words a partial takes back by a third, for about 0.1 s more before a word shows (the owner's
// choice, 2026-09-27). Finals are not affected: they only settle words well before the newest.
//
// Lessons from an earlier FluidAudio engine, each pinned by a test (LiveWindowTests):
//   - Its end-of-utterance model fired only on a pause: 25 s of unbroken speech produced no final
//     at all. Here the length bound settles unbroken speech too.
//   - Its callbacks carried the transcript accumulated since the session began, so every
//     utterance repeated the previous ones. Here a final carries only its own words and a partial
//     only the words not yet settled.
//   - Its force-commit timer was re-armed on every push, and pushes never stop (silence is audio
//     too), so it never fired. Here the bound is audio length since the utterance began, which no
//     push can move back.
//   - FluidAudio's `ASRResult.duration` is 0 for audio longer than one 15 s model window. Every
//     time here comes from sample counts; the decoder's duration is never read.

import InkBridge

/// 16 kHz: the only rate the core hands an engine.
let sampleRate = 16_000

/// Tuning of the trailing-window scheme, in samples and seconds.
public struct LiveWindowConfig: Sendable, Equatable {
    /// New audio between two decodes: how often the partial can change (0.5 s).
    public var hop = 8_000
    /// A gap this long after the last decoded word settles the utterance (0.8 s).
    public var pause = 12_800
    /// An utterance this long is settled at a word boundary, pause or not (12 s). It keeps every
    /// window inside one 15 s model pass.
    public var maxUtterance = 192_000
    /// When a long utterance is settled, words ending in its last stretch this long stay pending,
    /// so a word still being said is not cut (1 s).
    public var tailGuard = 16_000
    /// Audio kept when a decode hears no words (3 s). Parakeet often returns nothing for the first
    /// second or so of speech in a short window; with only 1 s kept, that speech was dropped before
    /// a longer window could hear it, and the live finals' WER on AMI rose from 22 % to 35 %.
    public var keepSilence = 48_000
    /// The shortest window worth decoding: FluidAudio refuses less than 0.3 s.
    public var minimum = 4_800
    /// A settled utterance keeps this much audio after its last word, so the next window does not
    /// start inside that word's tail (0.2 s).
    public var afterWord = 3_200
    /// A partial does not show words that end within this much of the decoded window's end
    /// (0.16 s): the newest words, which the next decode most often changes. The one place this
    /// policy is set; 0 shows every word.
    public var hideNewest = 2_560
    /// The most audio kept (30 s). Only a decoder that has fallen far behind lets the buffer grow
    /// past `maxUtterance`; then the oldest audio goes, so memory holds seconds, never the session
    /// (architecture rule 3). Its words are left to the final pass.
    public var maxBuffer = 480_000

    public init() {}
}

/// A decoded word, in samples from the start of the window it was decoded from.
public struct TimedWord: Sendable, Equatable {
    public var text: String
    public var start: Int
    public var end: Int

    public init(text: String, start: Int, end: Int) {
        self.text = text
        self.start = start
        self.end = end
    }
}

/// One window, decoded.
public struct DecodedWindow: Sendable, Equatable {
    /// The words in time order.
    public var words: [TimedWord]

    public init(words: [TimedWord]) {
        self.words = words
    }

    /// The words joined by single spaces.
    public var text: String {
        words.map(\.text).joined(separator: " ")
    }
}

/// A stretch of the stream to decode: its audio and where it sits, in samples from the stream's
/// first sample.
public struct Window: Sendable, Equatable {
    public var samples: [Float]
    public var start: Int

    public var end: Int { start + samples.count }
}

/// What the stream sends after a decode.
public enum LiveOutput: Sendable, Equatable {
    /// The words not settled yet, replacing the last partial ("" clears it).
    case partial(String)
    /// Settled words, in ms from the stream's first sample.
    case final(InkSegment)
}

/// The scheme's state for one stream. A value: the stream guards it.
public struct LiveWindow: Sendable {
    public let config: LiveWindowConfig
    /// Audio not settled yet, from `bufferStart`.
    private(set) var buffer: [Float] = []
    /// The stream sample `buffer[0]` is.
    private(set) var bufferStart = 0
    /// Samples received so far.
    public private(set) var received = 0
    /// `received` when the last window was taken.
    public private(set) var takenAt = 0
    /// The partial last sent.
    public private(set) var lastPartial = ""

    public init(config: LiveWindowConfig = LiveWindowConfig()) {
        self.config = config
    }

    /// Takes the stream's next samples.
    public mutating func append(_ samples: [Float]) {
        buffer.append(contentsOf: samples)
        received += samples.count
        if buffer.count > config.maxBuffer {
            drop(upTo: bufferStart + buffer.count - config.maxBuffer, unheard: true)
        }
    }

    /// Samples dropped unheard because the buffer outgrew `maxBuffer`.
    public private(set) var droppedUnheard = 0
    /// `droppedUnheard` when `takeUnheardDrops` last reported it.
    private var reportedUnheard = 0

    /// Samples dropped unheard since the last call. Every push past the cap drops some, so the
    /// stream asks once per decode (the one that fell that far behind) and logs one line for the
    /// whole stretch, never one per push.
    public mutating func takeUnheardDrops() -> Int {
        defer { reportedUnheard = droppedUnheard }
        return droppedUnheard - reportedUnheard
    }

    /// Audio received since the last window was taken: what a partial does not show yet.
    public var backlog: Int { received - takenAt }

    /// Whether enough new audio has arrived for another decode.
    public var wantsDecode: Bool {
        backlog >= config.hop && buffer.count >= config.minimum
    }

    /// The window to decode now: every sample not settled yet.
    public mutating func takeWindow() -> Window {
        takenAt = received
        return Window(samples: buffer, start: bufferStart)
    }

    /// Applies the decode of `window`, the window last taken. Audio that arrived meanwhile stays
    /// in the buffer for the next one.
    public mutating func apply(_ decoded: DecodedWindow, of window: Window) -> [LiveOutput] {
        // Audio dropped while the window decoded (the buffer outgrew its cap): the decode is stale.
        guard window.start == bufferStart else { return [] }
        let words = decoded.words
        guard let last = words.last else {
            // Nothing said: keep only the last moment of it, in front of whatever comes next.
            drop(upTo: max(bufferStart, window.end - config.keepSilence))
            return partial("")
        }
        if window.samples.count - last.end >= config.pause {
            // A pause after the last word: the utterance is settled.
            drop(upTo: window.start + min(last.end + config.afterWord, window.samples.count))
            return [.final(segment(words, in: window))] + partial("")
        }
        if window.samples.count >= config.maxUtterance {
            // Long, unbroken: settle up to a word boundary, leaving the words still being said.
            let limit = window.samples.count - config.tailGuard
            let settled = words.prefix { $0.end <= limit }
            let rest = words[settled.count...]
            guard let lastSettled = settled.last, let firstPending = rest.first else {
                // One word longer than the guard, or every word before it: settle everything.
                drop(upTo: window.start + min(last.end + config.afterWord, window.samples.count))
                return [.final(segment(words, in: window))] + partial("")
            }
            // Cut in the gap between the two words, so neither is split.
            drop(upTo: window.start + (lastSettled.end + firstPending.start) / 2)
            return [.final(segment(Array(settled), in: window))] + partial(shown(rest, in: window))
        }
        return partial(shown(words[...], in: window))
    }

    /// The window for the stream's end: everything not settled, when there is enough to decode.
    public mutating func takeLastWindow() -> Window? {
        takenAt = received
        guard buffer.count >= config.minimum else { return nil }
        return Window(samples: buffer, start: bufferStart)
    }

    /// Applies the last window's decode: every word is settled.
    public mutating func applyLast(_ decoded: DecodedWindow, of window: Window) -> [LiveOutput] {
        // Audio dropped while the window decoded (the buffer outgrew its cap): the decode is stale.
        guard window.start == bufferStart else { return [] }
        drop(upTo: window.end)
        guard !decoded.words.isEmpty else { return partial("") }
        return [.final(segment(decoded.words, in: window))] + partial("")
    }

    /// The stream's end with too little audio left to decode: nothing more is settled, and the
    /// partial is cleared.
    public mutating func endWithoutWindow() -> [LiveOutput] {
        drop(upTo: bufferStart + buffer.count)
        return partial("")
    }

    /// The words of a partial: those not ending in the newest `hideNewest` of `window`.
    private func shown(_ words: ArraySlice<TimedWord>, in window: Window) -> String {
        let limit = window.samples.count - config.hideNewest
        return words.prefix { $0.end <= limit }.map(\.text).joined(separator: " ")
    }

    /// The partial to send: `text`, unless it is what was sent last.
    private mutating func partial(_ text: String) -> [LiveOutput] {
        guard text != lastPartial else { return [] }
        lastPartial = text
        return [.partial(text)]
    }

    private mutating func drop(upTo sample: Int, unheard: Bool = false) {
        let n = min(max(0, sample - bufferStart), buffer.count)
        buffer.removeFirst(n)
        bufferStart += n
        if unheard { droppedUnheard += n }
    }

    /// Words of `window` as a final, in ms from the stream's first sample.
    private func segment(_ words: [TimedWord], in window: Window) -> InkSegment {
        let ms = { (sample: Int) in UInt64((window.start + sample) / (sampleRate / 1_000)) }
        return InkSegment(
            startMs: ms(words.first?.start ?? 0),
            endMs: ms(max(words.last?.end ?? 0, words.first?.start ?? 0)),
            text: words.map(\.text).joined(separator: " "))
    }
}
