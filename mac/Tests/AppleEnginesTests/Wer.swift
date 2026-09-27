// The word error rate the engine choice was measured with (the same rules as the core's Rust
// bench scorer, core/crates/ink-engines/tests/bench): lower-case, every character that is not a
// letter, mark or number (Unicode categories L, M, N) becomes a space, split on whitespace. WER is
// (substitutions + deletions + insertions) / reference words; a corpus sums edits and reference
// words over documents, never averaging per-document rates.
import XCTest

enum Wer {
    static func normalise(_ text: String) -> [String] {
        let kept = text.lowercased().unicodeScalars.map { scalar -> Character in
            if scalar == " " { return " " }
            switch scalar.properties.generalCategory {
            case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter,
                 .nonspacingMark, .spacingMark, .enclosingMark,
                 .decimalNumber, .letterNumber, .otherNumber:
                return Character(scalar)
            default:
                return " "
            }
        }
        return String(kept).split(whereSeparator: \.isWhitespace).map(String.init)
    }

    /// Edits for one pair, or summed over a corpus.
    struct Edits: Equatable, CustomStringConvertible {
        var reference = 0
        var edits = 0

        var wer: Double { reference == 0 ? .nan : 100 * Double(edits) / Double(reference) }

        static func + (a: Edits, b: Edits) -> Edits {
            Edits(reference: a.reference + b.reference, edits: a.edits + b.edits)
        }

        var description: String {
            String(format: "%d edits / %d words = %.2f %%", edits, reference, wer)
        }
    }

    /// Word-level Levenshtein distance between the normalised texts.
    static func score(reference: String, hypothesis: String) -> Edits {
        let r = normalise(reference)
        let h = normalise(hypothesis)
        var previous = Array(0...h.count)
        for (i, word) in r.enumerated() {
            var current = [i + 1] + Array(repeating: 0, count: h.count)
            for (j, other) in h.enumerated() {
                current[j + 1] = min(
                    previous[j] + (word == other ? 0 : 1),
                    previous[j + 1] + 1,
                    current[j] + 1)
            }
            previous = current
        }
        return Edits(reference: r.count, edits: previous[h.count])
    }
}

/// The scorer on cases worked out by hand.
final class WerTests: XCTestCase {
    func testTheNormaliserKeepsLettersMarksAndNumbersOnly() {
        XCTAssertEqual(Wer.normalise("Ready? Set -- GO: 3, 2, 1."), ["ready", "set", "go", "3", "2", "1"])
        XCTAssertEqual(Wer.normalise("Crème brûlée"), ["crème", "brûlée"])
        XCTAssertEqual(Wer.normalise("uh-huh, it's fine"), ["uh", "huh", "it", "s", "fine"])
        XCTAssertEqual(Wer.normalise("  \n\t "), [])
    }

    func testEditsAreCountedAndSummedOverACorpus() {
        // Five reference words: "cold" for "warm" (1) and "day" missing (1).
        let a = Wer.score(reference: "A warm and sunny day.", hypothesis: "a cold and sunny")
        XCTAssertEqual(a, Wer.Edits(reference: 5, edits: 2))
        // Three reference words and one inserted.
        let b = Wer.score(reference: "see you soon", hypothesis: "see you very soon")
        XCTAssertEqual(b, Wer.Edits(reference: 3, edits: 1))
        // The corpus is 3 edits over 8 words (37.5 %), not the mean of 40 % and 33.3 %.
        XCTAssertEqual((a + b).wer, 37.5, accuracy: 1e-9)
        XCTAssertEqual(Wer.score(reference: "one two", hypothesis: "").wer, 100)
        XCTAssertEqual(Wer.score(reference: "same words", hypothesis: "Same, words!").wer, 0)
    }

    /// The harness's flicker count: words shown that the next display drops or changes.
    func testFlickerCountsChangedWordsNotInsertions() {
        XCTAssertEqual(changedWords(["so", "the", "plan"], ["so", "the", "plan", "is"]), 0, "extended")
        XCTAssertEqual(changedWords(["so", "the", "plan"], ["so", "then", "the", "plan"]), 0, "a word inserted")
        XCTAssertEqual(changedWords(["so", "the", "plan"], ["so", "a", "plan"]), 1, "one changed")
        XCTAssertEqual(changedWords(["so", "the", "plan"], ["so"]), 2, "two dropped")
        XCTAssertEqual(changedWords([], ["new"]), 0)
    }
}
