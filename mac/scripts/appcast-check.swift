// Checks a Sparkle appcast the way an installed Inkwell will read it, before it is published
// (appcast.sh runs it; the release workflow through that). Run with `swift`, no package:
//
//   swift appcast-check.swift verify <appcast.xml> <dmg> <version> <url> <public-key> [--allow-unsigned-item]
//       The feed's own EdDSA signature, and the item for <version>: its download URL is <url>,
//       its length is the dmg's size, and its EdDSA signature is the dmg's, all against
//       <public-key> (base64, the app's SUPublicEDKey). Exit 1 on any mismatch.
//       --allow-unsigned-item: the rehearsal's app may carry no key, and then Sparkle signs no
//       item; everything else is still checked.
//   swift appcast-check.swift feed <appcast.xml> <public-key>
//       Only the feed's own signature (a published feed, before its items are carried over).
//   swift appcast-check.swift signature <file> <signature> <public-key>
//       One EdDSA signature over a file.
//   swift appcast-check.swift throwaway-key <private-key-file>
//       A fresh key for the rehearsal only: writes its private half (mode 0600, the form
//       generate_appcast reads) and prints its public half. Never a release key: that one is the
//       maintainer's, made with Sparkle's generate_keys (docs/RELEASING.md).
//
// Sparkle's EdDSA is Ed25519 over the file's bytes; a signed feed carries its signature in a
// trailing comment, over the bytes before that comment ("length"). CryptoKit's Curve25519
// signing is Ed25519, so nothing but the system is needed here.
import CryptoKit
import Foundation

struct CheckFailed: Error, CustomStringConvertible {
    let description: String
}

func fail(_ message: String) -> CheckFailed { CheckFailed(description: message) }

func publicKey(_ base64: String) throws -> Curve25519.Signing.PublicKey {
    guard let raw = Data(base64Encoded: base64), raw.count == 32 else {
        throw fail("the public key is not 32 bytes of base64")
    }
    return try Curve25519.Signing.PublicKey(rawRepresentation: raw)
}

func signatureIsValid(_ base64: String, over data: Data, key: Curve25519.Signing.PublicKey) -> Bool {
    guard let signature = Data(base64Encoded: base64) else { return false }
    return key.isValidSignature(signature, for: data)
}

/// The feed's trailing signature comment, checked over the bytes before it.
func checkFeedSignature(_ feed: Data, key: Curve25519.Signing.PublicKey) throws {
    let marker = Data("<!-- sparkle-signatures:".utf8)
    guard let range = feed.range(of: marker, options: .backwards) else {
        throw fail("the feed is not signed (no sparkle-signatures comment)")
    }
    let trailer = String(decoding: feed[range.lowerBound...], as: UTF8.self)
    func field(_ name: String) -> String? {
        trailer.split(separator: "\n")
            .first { $0.hasPrefix("\(name): ") }
            .map { String($0.dropFirst(name.count + 2)).trimmingCharacters(in: .whitespaces) }
    }
    guard let signature = field("edSignature"), let length = field("length").flatMap(Int.init) else {
        throw fail("the feed's signature comment has no edSignature or length")
    }
    // The signed bytes end where the comment starts: nothing may sit between them unsigned.
    guard length == range.lowerBound else {
        throw fail("the feed's signature covers \(length) bytes, but its comment starts at \(range.lowerBound)")
    }
    guard signatureIsValid(signature, over: feed[..<range.lowerBound], key: key) else {
        throw fail("the feed's signature does not verify against the app's key")
    }
}

func verify(appcast: String, dmg: String, version: String, url: String, key keyBase64: String,
            allowUnsignedItem: Bool) throws {
    let key = try publicKey(keyBase64)
    let feed = try Data(contentsOf: URL(fileURLWithPath: appcast))
    try checkFeedSignature(feed, key: key)
    print("feed: signed, and the signature verifies")

    let document = try XMLDocument(data: feed)
    let items = try document.nodes(forXPath: "/rss/channel/item").compactMap { $0 as? XMLElement }
    // Namespaced children by qualified name: the feed declares sparkle: itself.
    let matching = items.filter {
        $0.elements(forName: "sparkle:version").first?.stringValue == version
    }
    guard matching.count == 1, let item = matching.first else {
        throw fail("the feed has \(matching.count) items for version \(version), not one")
    }
    guard let enclosure = item.elements(forName: "enclosure").first else {
        throw fail("the item for \(version) has no enclosure")
    }
    guard enclosure.attribute(forName: "url")?.stringValue == url else {
        throw fail("the item's URL is \(enclosure.attribute(forName: "url")?.stringValue ?? "missing"), not \(url)")
    }
    let archive = try Data(contentsOf: URL(fileURLWithPath: dmg), options: .mappedIfSafe)
    guard enclosure.attribute(forName: "length")?.stringValue == String(archive.count) else {
        throw fail("the item's length is not the dmg's size (\(archive.count) bytes)")
    }
    guard let signature = enclosure.attribute(forName: "sparkle:edSignature")?.stringValue else {
        if allowUnsignedItem {
            print("item \(version): URL and length match; not signed (allowed in the rehearsal)")
            return
        }
        throw fail("the item for \(version) has no EdDSA signature: does the app carry SUPublicEDKey?")
    }
    guard signatureIsValid(signature, over: archive, key: key) else {
        throw fail("the item's signature does not verify against the app's key")
    }
    print("item \(version): URL and length match, and its signature verifies against the app's key")
}

func throwawayKey(to path: String) throws {
    let key = Curve25519.Signing.PrivateKey()
    let created = FileManager.default.createFile(
        atPath: path, contents: Data(key.rawRepresentation.base64EncodedString().utf8),
        attributes: [.posixPermissions: 0o600])
    guard created else { throw fail("could not write \(path)") }
    print(key.publicKey.rawRepresentation.base64EncodedString())
}

let args = Array(CommandLine.arguments.dropFirst())
do {
    switch (args.first, args.count) {
    case ("verify", 6), ("verify", 7):
        let allow = args.count == 7
        if allow, args[6] != "--allow-unsigned-item" { throw fail("unknown argument: \(args[6])") }
        try verify(appcast: args[1], dmg: args[2], version: args[3], url: args[4], key: args[5],
                   allowUnsignedItem: allow)
    case ("feed", 3):
        try checkFeedSignature(try Data(contentsOf: URL(fileURLWithPath: args[1])), key: try publicKey(args[2]))
        print("feed: signed, and the signature verifies")
    case ("signature", 4):
        let data = try Data(contentsOf: URL(fileURLWithPath: args[1]), options: .mappedIfSafe)
        guard signatureIsValid(args[2], over: data, key: try publicKey(args[3])) else {
            throw fail("the signature does not verify")
        }
        print("signature: verifies")
    case ("throwaway-key", 2):
        try throwawayKey(to: args[1])
    default:
        throw fail("usage: see the head of appcast-check.swift")
    }
} catch {
    FileHandle.standardError.write(Data("appcast-check: \(error)\n".utf8))
    exit(1)
}
