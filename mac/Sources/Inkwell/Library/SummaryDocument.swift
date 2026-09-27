// A summary as the screen shows it. The core stores summaries as markdown (a headline, the body in
// short sections, then Decisions, Actions and Open questions as lists); the earlier app printed
// that raw, `##` and `**` and all. Here it becomes blocks of styled text, and no markdown token
// survives into what is drawn.
//
// Blocks are read line by line (headings, bullet and numbered lists, quotes, rules, code fences,
// paragraphs); inline markup (bold, italic, code, strikethrough) is Foundation's markdown
// parser's, inline only. Whatever the parser refuses is stripped of its markers instead: the
// reader sees the words, never the syntax.
//
// Links are words only. A summary is written by a language model from what the far end said, so
// a link in it is untrusted: its text stays, its destination is dropped (no run carries a link),
// and the views that show a summary refuse to open one anyway (SummaryLinks).
import Foundation
import SwiftUI

/// One block of a rendered summary.
enum SummaryBlock: Equatable, Sendable {
    /// A section heading (`##`), level 1 to 6.
    case heading(level: Int, text: AttributedString)
    /// A paragraph.
    case paragraph(AttributedString)
    /// A list item; `marker` is `•` or the item's number (`1.`).
    case item(marker: String, text: AttributedString)
}

/// A rendered summary: its headline (the first paragraph, which is also the record's title) and
/// the blocks after it.
struct SummaryDocument: Equatable, Sendable {
    let headline: AttributedString?
    let blocks: [SummaryBlock]

    /// Renders `markdown`.
    init(markdown: String) {
        var blocks = Self.parse(markdown)
        if case .paragraph(let first)? = blocks.first {
            headline = first
            blocks.removeFirst()
        } else {
            headline = nil
        }
        self.blocks = blocks
    }

    /// The whole summary as plain text, as drawn: what Copy puts on the pasteboard, and what the
    /// snapshot test reads for markdown tokens.
    var plainText: String {
        var lines: [String] = []
        if let headline {
            lines.append(String(headline.characters))
        }
        for block in blocks {
            switch block {
            case .heading(_, let text):
                lines.append("")
                lines.append(String(text.characters))
            case .paragraph(let text):
                lines.append("")
                lines.append(String(text.characters))
            case .item(let marker, let text):
                lines.append("\(marker) \(String(text.characters))")
            }
        }
        return lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The first paragraph after the headline, for the Today card: a sentence or two, no lists.
    var lede: AttributedString? {
        for block in blocks {
            if case .paragraph(let text) = block { return text }
        }
        return nil
    }

    // MARK: Blocks

    private static func parse(_ markdown: String) -> [SummaryBlock] {
        var blocks: [SummaryBlock] = []
        var paragraph: [String] = []
        var inFence = false
        func flush() {
            let text = paragraph.joined(separator: " ").trimmingCharacters(in: .whitespaces)
            if !text.isEmpty {
                blocks.append(.paragraph(inline(text)))
            }
            paragraph.removeAll()
        }
        for raw in markdown.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("```") || line.hasPrefix("~~~") {
                flush()
                inFence.toggle()
                continue
            }
            if inFence {
                // Code is shown as its text, one paragraph per line.
                if !line.isEmpty { blocks.append(.paragraph(AttributedString(line))) }
                continue
            }
            if line.isEmpty {
                flush()
                continue
            }
            if isRule(line) {
                flush()
                continue
            }
            if let (level, text) = heading(line) {
                flush()
                blocks.append(.heading(level: level, text: inline(text)))
                continue
            }
            if let text = bullet(line) {
                flush()
                blocks.append(.item(marker: "•", text: inline(text)))
                continue
            }
            if let (number, text) = numbered(line) {
                flush()
                blocks.append(.item(marker: "\(number).", text: inline(text)))
                continue
            }
            if line.hasPrefix(">") {
                paragraph.append(String(line.drop(while: { $0 == ">" || $0 == " " })))
                continue
            }
            paragraph.append(line)
        }
        flush()
        return blocks
    }

    private static func isRule(_ line: String) -> Bool {
        let chars = Set(line.filter { $0 != " " })
        return line.filter({ $0 != " " }).count >= 3 && chars.count == 1
            && ["-", "*", "_"].contains(chars.first.map(String.init) ?? "")
    }

    private static func heading(_ line: String) -> (Int, String)? {
        let hashes = line.prefix(while: { $0 == "#" }).count
        guard (1...6).contains(hashes) else { return nil }
        let rest = line.dropFirst(hashes)
        guard rest.first == " " else { return nil }
        let text = rest.trimmingCharacters(in: .whitespaces)
        // A closing run of #s is part of the syntax too.
        return (hashes, text.replacingOccurrences(of: #"\s+#+$"#, with: "", options: .regularExpression))
    }

    private static func bullet(_ line: String) -> String? {
        for marker in ["- ", "* ", "+ ", "• "] where line.hasPrefix(marker) {
            var text = String(line.dropFirst(marker.count))
            // A task list's box is syntax as well.
            for box in ["[ ] ", "[x] ", "[X] "] where text.hasPrefix(box) {
                text = String(text.dropFirst(box.count))
            }
            return text
        }
        return nil
    }

    private static func numbered(_ line: String) -> (Int, String)? {
        let digits = line.prefix(while: \.isNumber)
        guard !digits.isEmpty, digits.count <= 3, let number = Int(digits) else { return nil }
        let rest = line.dropFirst(digits.count)
        guard rest.hasPrefix(". ") || rest.hasPrefix(") ") else { return nil }
        return (number, String(rest.dropFirst(2)))
    }

    // MARK: Inline

    /// Bold, italic, code, links and strikethrough become attributes; their markers go.
    static func inline(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(
            allowsExtendedAttributes: false,
            interpretedSyntax: .inlineOnlyPreservingWhitespace,
            failurePolicy: .returnPartiallyParsedIfPossible)
        if var parsed = try? AttributedString(markdown: text, options: options),
            !containsMarkup(String(parsed.characters))
        {
            // The words of a link stay; where it goes does not.
            for run in parsed.runs where run.link != nil {
                parsed[run.range].link = nil
            }
            return parsed
        }
        return AttributedString(stripped(text))
    }

    /// Whether `text` still holds inline markup: paired emphasis markers, code ticks, a link.
    static func containsMarkup(_ text: String) -> Bool {
        text.contains("**") || text.contains("__") || text.contains("`") || text.contains("~~")
            || text.range(of: #"\]\([^)]*\)"#, options: .regularExpression) != nil
    }

    /// `text` with inline markers removed, keeping the words (and a link's text).
    static func stripped(_ text: String) -> String {
        var out = text.replacingOccurrences(of: #"!?\[([^\]]*)\]\([^)]*\)"#, with: "$1", options: .regularExpression)
        for marker in ["**", "__", "~~", "`"] {
            out = out.replacingOccurrences(of: marker, with: "")
        }
        // Single * or _ around a word (italic), not an apostrophe or a snake_case name.
        out = out.replacingOccurrences(of: #"(?<![\w*])[*_](\S(?:.*?\S)?)[*_](?![\w*])"#, with: "$1", options: .regularExpression)
        return out
    }
}

/// What the views showing a summary do with a link: refuse it. The summary's runs carry none, so
/// this is the second line: nothing a summary holds opens a URL.
enum SummaryLinks {
    enum Decision: Equatable {
        case refused
    }

    static func decide(_ url: URL) -> Decision {
        .refused
    }

    /// The openURL action for those views.
    @MainActor static var action: OpenURLAction {
        OpenURLAction { url in
            _ = decide(url)
            return .discarded
        }
    }
}

extension View {
    /// Refuses to open any link from within: for views that show a summary's words.
    func refusingLinks() -> some View {
        environment(\.openURL, SummaryLinks.action)
    }
}
