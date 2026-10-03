// Settings > AI's language model you bring (own key): a provider (OpenAI, Anthropic, Groq,
// OpenRouter, or any OpenAI-compatible server), its API key, and a model; and the local-only
// switch.
//
// The core holds everything (its llm.* commands): llm.providers says which providers there are,
// whether each has a key stored (asked without reading it), which one is chosen and whether it is
// ready. The key goes once, in llm.key.save, into the macOS keychain; this model never keeps it,
// and nothing here logs it.
//
// A provider chosen here comes first: polish, voice edit, summaries and Ask use it. With none
// chosen they use Apple Intelligence, while it is available.
//
// Choosing a provider that is not on this Mac turns local-only mode off, and the step says so
// before the user presses Use; the command carries the user's say-so, and the core refuses such a
// choice without it. Choosing sends nothing: each feature sends only with its own consent for that
// provider (ConsentModel), and the core refuses to send without it. Test sends one short fixed
// request (never the user's words) with the key. A failure of any command is said under the
// section ("Couldn't ..."), never read as success.
import Foundation
import InkBridge
import Observation

@MainActor
@Observable
final class CloudModel {
    /// A provider as the picker shows it.
    struct Provider: Equatable, Identifiable, Sendable {
        /// The core's id (openai, groq, anthropic, openrouter, custom).
        let id: String
        let name: String
        /// The model used when none is named.
        let defaultModel: String
        /// Its address (for custom, the default one).
        let endpoint: String
        /// Whether the user names the address (custom only).
        let customURL: Bool
        let needsKey: Bool
        let hasKey: Bool
    }

    /// Where a key test is.
    enum TestState: Equatable, Sendable {
        case none
        case testing
        case passed
        case failed
    }

    // What the core says.

    /// The providers, in the core's order (empty until read).
    private(set) var providers: [Provider] = []
    /// Whether the core has answered once.
    private(set) var loaded = false
    /// The chosen provider's id (nil: none).
    private(set) var chosen: String?
    private(set) var chosenModel: String?
    /// For a custom server, its address as chosen.
    private(set) var chosenBaseURL: String?
    /// Whether the chosen provider is off this Mac.
    private(set) var chosenIsCloud = false
    /// Local-only mode: on unless the user turned it off (or chose a provider off this Mac).
    private(set) var localOnly = true
    /// Whether the chosen provider can be called (its key stored, local-only mode letting it
    /// through).
    private(set) var ready = false
    /// What the core could not read ("couldn't read ...").
    private(set) var readError: String?

    // The picker (the provider being set up: the chosen one until the user picks another).

    /// The provider in the picker (nil: none).
    private(set) var selected: String?
    /// The model in the picker; empty for the provider's default.
    var draftModel = ""
    /// A custom server's address in the picker.
    var draftBaseURL = ""
    /// A command that failed, as a sentence.
    private(set) var failure: String?
    private(set) var testState = TestState.none
    /// What the last test said.
    private(set) var testMessage: String?

    @ObservationIgnored private let send: SendCommand
    @ObservationIgnored private var requests = 0
    /// The newest test's ref: only its answer is shown, and none once the provider, model or key
    /// changed.
    @ObservationIgnored private var testRef: String?

    init(send: @escaping SendCommand) {
        self.send = send
    }

    /// Every ref this model sends starts with this.
    static let refPrefix = "llm."
    /// The local-only switch's command id.
    static let localOnlySettingID = "setting:\(ShellSetting.llmLocalOnly.rawValue)"

    /// The provider in the picker, if the core listed it.
    var selectedProvider: Provider? { providers.first { $0.id == selected } }
    var chosenProvider: Provider? { providers.first { $0.id == chosen } }

    /// A name for people.
    static func name(_ id: String) -> String {
        switch id {
        case "openai": "OpenAI"
        case "anthropic": "Anthropic"
        case "groq": "Groq"
        case "openrouter": "OpenRouter"
        case "custom": "OpenAI-compatible server"
        default: id
        }
    }

    /// Whether the provider in the picker, with the address typed, is off this Mac.
    var selectedIsCloud: Bool {
        guard let p = selectedProvider else { return false }
        return !p.customURL || !Self.isOnThisMac(baseURL(for: p))
    }

    /// Where the provider in the picker sends, with the address typed (nil: none picked), as the
    /// core names a destination: without the trailing slashes it drops (ink-llm's EndpointUrl).
    var selectedEndpoint: String? {
        selectedProvider.map { p in
            var url = Substring(baseURL(for: p))
            while url.hasSuffix("/") { url = url.dropLast() }
            return String(url)
        }
    }

    /// Puts `id` in the picker while nothing is chosen or picked (the first run points at Groq's
    /// free key). Nothing is sent.
    func suggest(_ id: String) {
        guard chosen == nil, selected == nil, providers.contains(where: { $0.id == id }) else { return }
        select(id)
    }

    /// The first run's own key is one choice, Groq's free model (its key and Use); another
    /// provider or model is behind "Other providers or models…". Those open first when another
    /// provider is chosen, or picked here or in Settings > AI: the user's pick stands.
    var firstRunStartsOnOthers: Bool { selected != nil && selected != "groq" }

    /// Back from the other providers to Groq's free model: Groq in the picker. Nothing is sent,
    /// and nothing chosen changes until Use.
    func pickGroq() {
        guard providers.contains(where: { $0.id == "groq" }) else { return }
        select("groq")
    }

    /// Whether Use would change anything: another provider, model or address than the chosen one.
    var canUse: Bool {
        guard loaded else { return false }
        guard let p = selectedProvider else { return selected == nil && chosen != nil }
        if p.customURL, draftBaseURL.trimmingCharacters(in: .whitespaces).isEmpty { return false }
        return p.id != chosen || model(for: p) != chosenModel || (p.customURL && baseURL(for: p) != chosenBaseURL)
    }

    /// Whether Test can be pressed: the provider in the picker is the chosen one, and no test is
    /// running.
    var canTest: Bool { chosen != nil && selected == chosen && testState != .testing }

    /// What the Use button says.
    var useLabel: String {
        selectedProvider.map { "Use \(Self.name($0.id))" } ?? "Stop using a language model"
    }

    /// What choosing the provider in the picker means, said before the user presses Use.
    var useNote: String {
        guard let p = selectedProvider else {
            return chosen == nil
                ? "No language model of your own is chosen. Nothing you say leaves this Mac."
                : "Stopping turns local-only mode back on: nothing you say leaves this Mac."
        }
        let name = Self.name(p.id)
        return selectedIsCloud
            ? "Using \(name) turns local-only mode off, so the features below can send to \(name). Each one sends only once you allow it for \(name); one you already allowed for \(name) sends again straight away."
            : "This server is on this Mac, so local-only mode stays on. Each feature below uses it only once you allow it; one you already allowed uses it straight away."
    }

    /// What Use means in the first run, where it asks polish's consent before choosing.
    var firstRunUseNote: String {
        guard let p = selectedProvider else { return "Pick a provider to bring your own key." }
        let name = Self.name(p.id)
        if p.needsKey && !p.hasKey { return "Save your \(name) key first." }
        return selectedIsCloud
            ? "Use asks first: polish sends your words to \(name) only once you allow it, which also turns Local only off."
            : "This server is on this Mac, so Local only stays on. Use asks before polish uses it."
    }

    /// Whether the core keeps the key back from the server in the picker: a custom server over
    /// plain http that is not on this Mac gets no key, stored or not (the core's rule).
    var keyWithheld: Bool {
        guard let p = selectedProvider, p.customURL else { return false }
        return Self.keyWithheld(from: baseURL(for: p))
    }

    /// The key's name for people: "Groq key"; a server of the user's own has a "server key".
    static func keyName(_ p: Provider) -> String {
        p.customURL ? "server key" : "\(name(p.id)) key"
    }

    /// "A Groq key", "An OpenAI key".
    private static func aKey(_ p: Provider) -> String {
        let key = keyName(p)
        // By the first letter, whatever its case: an id this build has no name for is lower case.
        return ("aeiou".contains(key.prefix(1).lowercased()) ? "An " : "A ") + key
    }

    /// The line about the key of the provider in the picker. Keys are in the keychain of the Mac
    /// account, shared by every Inkwell on it and never in a library: a scratch library sees the
    /// user's own key, so the line says whose it is.
    var keyStatus: String {
        guard let p = selectedProvider else { return "" }
        if p.hasKey && keyWithheld {
            return "\(Self.aKey(p)) is already saved in your keychain for this Mac account, but it is not sent to this server: keys go only over https or to a server on this Mac."
        }
        if keyWithheld { return "No key is sent to this server: keys go only over https or to a server on this Mac." }
        if !p.needsKey && !p.hasKey { return "No key is needed unless your server asks for one." }
        return p.hasKey
            ? "\(Self.aKey(p)) is already saved in your keychain for this Mac account."
            : "No \(Self.keyName(p)) is saved yet."
    }

    /// The Delete button of the provider in the picker: what it deletes, and that it asks first.
    var deleteKeyLabel: String {
        selectedProvider.map { "Delete \(Self.keyName($0))\u{2026}" } ?? "Delete key\u{2026}"
    }

    /// The question Delete asks about `id`'s key (the provider it was pressed for).
    func deleteKeyTitle(_ id: String?) -> String {
        providers.first { $0.id == id }.map { "Delete the \(Self.keyName($0)) from your keychain?" }
            ?? "Delete the key from your keychain?"
    }

    static let deleteKeyMessage = "It is saved in your keychain for this Mac account, not in this library: every Inkwell on this account stops using it. You can paste it again later."

    /// The line about what is in use now.
    var status: String {
        if let readError { return "\(Self.sentence(readError))." }
        guard loaded else { return "Reading the language model settings…" }
        guard let p = chosenProvider else {
            return localOnly
                ? "No language model of your own is in use. Local-only mode is on."
                : "No language model of your own is in use."
        }
        let name = Self.name(p.id)
        let model = chosenModel ?? p.defaultModel
        let where_ = chosenIsCloud ? name : "\(name), on this Mac"
        if p.needsKey && !p.hasKey {
            return "\(model) at \(where_) is chosen, but no key is stored, so nothing can use it."
        }
        if !ready {
            return "\(model) at \(where_) is chosen, but local-only mode is on, so nothing is sent."
        }
        return chosenIsCloud
            ? "In use when Apple Intelligence isn't: \(model) at \(name). Local-only mode is off."
            : "In use when Apple Intelligence isn't: \(model) at \(where_). Local-only mode is on."
    }

    /// Whether the status is a problem to show in the alert colour.
    var isProblem: Bool { failure != nil || readError != nil || testState == .failed }

    // MARK: - Commands

    private func nextRef(_ kind: String) -> String {
        requests += 1
        return "\(Self.refPrefix)\(kind):\(requests)"
    }

    /// Reads the providers from the core.
    func load() {
        send(.llmProviders(ref: nextRef("providers")))
    }

    /// The user picked a provider (nil: none) in the picker. Nothing is sent.
    func select(_ id: String?) {
        forgetTest()
        selected = id
        draftModel = id != nil && id == chosen ? chosenModel ?? "" : ""
        draftBaseURL = id != nil && id == chosen ? chosenBaseURL ?? "" : ""
        if id != nil, draftBaseURL.isEmpty, let p = selectedProvider, p.customURL {
            draftBaseURL = p.endpoint
        }
        failure = nil
    }

    /// Stores `key` for the provider in the picker. Sent once, not kept: the view clears its field.
    func saveKey(_ key: String) {
        guard let p = selectedProvider else { return }
        guard !key.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            failure = "Type or paste the key first."
            return
        }
        failure = nil
        forgetTest()
        send(.llmKeySave(provider: p.id, key: key, ref: nextRef("key.save")))
    }

    /// Deletes the stored key of `id`: the provider Delete was pressed for, so a confirmation
    /// answered after the picker moved (Settings > AI shares it) deletes what it named.
    func deleteKey(_ id: String) {
        guard providers.contains(where: { $0.id == id }) else { return }
        failure = nil
        forgetTest()
        send(.llmKeyDelete(provider: id, ref: nextRef("key.delete")))
    }

    /// Use: chooses the provider in the picker with its model (and address), or none. For a
    /// provider off this Mac it carries the user's say-so that local-only mode goes off. Returns
    /// whether it sent the choice.
    @discardableResult
    func use() -> Bool {
        guard canUse else { return false }
        failure = nil
        forgetTest()
        guard let p = selectedProvider else {
            send(.llmChoose(provider: "none", model: nil, baseURL: nil, localOnlyOff: false, ref: nextRef("choose")))
            return true
        }
        let model = draftModel.trimmingCharacters(in: .whitespaces)
        send(.llmChoose(
            provider: p.id, model: model.isEmpty ? nil : model, baseURL: p.customURL ? baseURL(for: p) : nil,
            localOnlyOff: selectedIsCloud, ref: nextRef("choose")))
        return true
    }

    /// Test: one short fixed request to the chosen provider.
    func test() {
        guard canTest, let chosen else { return }
        let ref = nextRef("test")
        testRef = ref
        testState = .testing
        testMessage = "Asking \(Self.name(chosen))…"
        send(.llmTest(ref: ref))
    }

    /// The local-only switch: on, no model off this Mac is called, whatever is chosen.
    func setLocalOnly(_ on: Bool) {
        failure = nil
        localOnly = on
        send(.settingSet(.llmLocalOnly, on ? "on" : "off"))
    }

    /// The provider, model or key changed: a test in flight or done was of what came before.
    private func forgetTest() {
        testRef = nil
        testState = .none
        testMessage = nil
    }

    /// Whether this model shows `failed`: its own commands (their ids are its refs), and the
    /// local-only switch's.
    static func handles(_ failed: CommandFailed) -> Bool {
        if failed.id == localOnlySettingID { return true }
        return failed.command.hasPrefix("llm.") && (failed.id?.hasPrefix(refPrefix) ?? false)
    }

    func apply(_ event: InkEvent) {
        switch event {
        case .llmProviders(let state):
            let first = !loaded
            providers = state.providers.map {
                Provider(id: $0.id, name: Self.name($0.id), defaultModel: $0.defaultModel, endpoint: $0.endpoint,
                         customURL: $0.customUrl, needsKey: $0.needsKey, hasKey: $0.hasKey)
            }
            let choiceChanged = chosen != state.chosen || chosenModel != state.model || chosenBaseURL != state.baseUrl
            chosen = state.chosen
            chosenModel = state.model
            chosenBaseURL = state.baseUrl
            chosenIsCloud = state.to == .cloud
            localOnly = state.localOnly
            ready = state.ready
            readError = state.error
            loaded = true
            // The picker follows the choice when it is first read and whenever it changes.
            if first || choiceChanged {
                select(chosen)
            }
        case .llmTested(let tested) where tested.ref != nil && tested.ref == testRef:
            testState = tested.ok ? .passed : .failed
            testMessage = tested.ok
                ? "\(Self.name(tested.provider)) answered with \(tested.model)."
                : "\(Self.sentence(tested.error ?? "couldn't get an answer"))."
        case .commandFailed(let failed) where Self.handles(failed):
            if failed.id == Self.localOnlySettingID {
                failure = "Couldn't change local-only mode: \(failed.message)."
                load()
            } else if failed.command == "llm.test" {
                guard failed.id == testRef else { return }
                testState = .failed
                testMessage = "Couldn't test it: \(failed.message)."
            } else {
                // The core's words, without the command's name it starts a refusal with.
                let prefix = "\(failed.command): "
                let message = failed.message.hasPrefix(prefix) ? String(failed.message.dropFirst(prefix.count)) : failed.message
                failure = message.hasPrefix("couldn't") ? "\(Self.sentence(message))." : "Couldn't do that: \(message)."
            }
        case .settingValue(let value) where value.key == ShellSetting.llmLocalOnly.rawValue:
            // Local-only mode changed: whether the chosen provider can be called changed with it.
            load()
        case .coreStopped:
            if testState == .testing {
                testState = .none
                testMessage = nil
            }
        default:
            break
        }
    }

    private func model(for p: Provider) -> String {
        let model = draftModel.trimmingCharacters(in: .whitespaces)
        return model.isEmpty ? p.defaultModel : model
    }

    private func baseURL(for p: Provider) -> String {
        let url = draftBaseURL.trimmingCharacters(in: .whitespaces)
        return url.isEmpty ? p.endpoint : url
    }

    /// Whether the core keeps a key back from `url`: plain http to a server that is not on this
    /// Mac.
    static func keyWithheld(from url: String) -> Bool {
        url.trimmingCharacters(in: .whitespaces).lowercased().hasPrefix("http://") && !isOnThisMac(url)
    }

    /// Whether a server address is on this Mac, by the core's rule: host localhost, an IPv4
    /// address in 127.0.0.0/8 written as four plain numbers, or [::1]. Anything else, or anything
    /// that does not read, is off this Mac (the core decides in the end, and refuses a choice that
    /// disagrees).
    static func isOnThisMac(_ url: String) -> Bool {
        let text = url.trimmingCharacters(in: .whitespaces)
        guard let scheme = text.range(of: "://"),
              ["http", "https"].contains(text[..<scheme.lowerBound].lowercased())
        else { return false }
        var authority = text[scheme.upperBound...]
        if let end = authority.firstIndex(where: { "/?#".contains($0) }) {
            authority = authority[..<end]
        }
        guard !authority.contains("@") else { return false }
        if authority.hasPrefix("[") {
            guard let close = authority.firstIndex(of: "]") else { return false }
            let host = authority[authority.index(after: authority.startIndex)..<close]
            var address = in6_addr()
            guard inet_pton(AF_INET6, String(host), &address) == 1 else { return false }
            return withUnsafeBytes(of: &address) { Array($0) } == Array(repeating: 0, count: 15) + [1]
        }
        let host = authority.split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false).first ?? ""
        if host.lowercased() == "localhost" { return true }
        let parts = host.split(separator: ".", omittingEmptySubsequences: false)
        return parts.count == 4 && parts[0] == "127" && parts.allSatisfy { part in
            (1...3).contains(part.count) && part.allSatisfy { $0.isASCII && $0.isNumber }
                && (part.count == 1 || part.first != "0") && (Int(part) ?? 256) <= 255
        }
    }

    /// "couldn't x" as the start of a sentence.
    private static func sentence(_ s: String) -> String {
        s.isEmpty ? s : s.prefix(1).uppercased() + s.dropFirst()
    }
}
