// The library's queries as the core reads them (inkwell.h lists them). Each goes out with an id;
// the answer echoes it as "request", which is how LibraryModel matches answers to questions and
// drops an answer a newer question has made stale.
import Foundation
import InkBridge

/// One library query.
enum LibraryCommand: Equatable, Sendable {
    /// Where a page of records continues: the last record of the previous page.
    struct Cursor: Equatable, Sendable {
        let startedAtUnixMs: Int64
        let record: String
    }

    case list(kind: RecordKind?, before: Cursor?, limit: Int)
    case search(query: String, limit: Int)
    case open(record: String)
    case owed(limit: Int)
    case setDone(commitment: String, done: Bool)
    case stats(sinceUnixMs: Int64)
    case permissions

    /// The command's name, as the core reads it.
    var name: String {
        switch self {
        case .list: "records.list"
        case .search: "records.search"
        case .open: "record.open"
        case .owed: "commitments.open"
        case .setDone: "commitment.set_done"
        case .stats: "library.stats"
        case .permissions: "permissions.check"
        }
    }

    /// The command as JSON, carrying `id`.
    func json(id: String) -> String {
        var fields: [String: Any] = ["cmd": name, "id": id]
        switch self {
        case .list(let kind, let before, let limit):
            fields["limit"] = limit
            if let kind { fields["kind"] = kind.rawValue }
            if let before {
                fields["before"] = ["started_at_unix_ms": before.startedAtUnixMs, "id": before.record]
            }
        case .search(let query, let limit):
            fields["query"] = query
            fields["limit"] = limit
        case .open(let record):
            fields["record"] = record
        case .owed(let limit):
            fields["limit"] = limit
        case .setDone(let commitment, let done):
            fields["commitment"] = commitment
            fields["done"] = done
        case .stats(let since):
            fields["since_unix_ms"] = since
        case .permissions:
            break
        }
        // Only strings, numbers, booleans and nested objects of them: serialisation cannot fail.
        let data = (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }
}
