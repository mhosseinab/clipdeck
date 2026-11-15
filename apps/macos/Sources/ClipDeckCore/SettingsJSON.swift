import Foundation

/// Minimal JSON value for preserving unknown settings keys across
/// load/save cycles (behavior.md §1.3).
public enum JSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([JSONValue].self) { self = .array(a) }
        else if let o = try? c.decode([String: JSONValue].self) { self = .object(o) }
        else { throw DecodingError.dataCorruptedError(in: c, debugDescription: "unsupported JSON value") }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }
}

private let knownSettingsKeys: Set<String> = [
    "watchEnabled", "soundCopy", "soundPaste", "maxItems", "shortcut",
    "launchAtLogin", "theme", "excludedApps", "ignoreTransient",
    "ignoreConfidential", "generateLinkPreviews", "pasteAsPlainText",
    "stackDirection", "plainTextApps", "syncDirectory", "showInScreenShare", "hooks",
]

extension Settings: Codable {
    public init(from decoder: Decoder) throws {
        let obj = try [String: JSONValue](from: decoder)
        self.init(fromJSONObject: obj)
    }

    public func encode(to encoder: Encoder) throws {
        var obj = extra
        func put(_ k: String, _ v: JSONValue) { obj[k] = v }
        put("watchEnabled", .bool(watchEnabled))
        put("soundCopy", .bool(soundCopy))
        put("soundPaste", .bool(soundPaste))
        put("maxItems", .number(Double(maxItems)))
        put("shortcut", .string(shortcut))
        put("launchAtLogin", .bool(launchAtLogin))
        put("theme", .string(theme))
        put("excludedApps", .array(excludedApps.map { .string($0) }))
        put("ignoreTransient", .bool(ignoreTransient))
        put("ignoreConfidential", .bool(ignoreConfidential))
        put("generateLinkPreviews", .bool(generateLinkPreviews))
        put("pasteAsPlainText", .bool(pasteAsPlainText))
        put("stackDirection", .string(stackDirection))
        put("plainTextApps", .array(plainTextApps.map { .string($0) }))
        put("syncDirectory", .string(syncDirectory))
        put("showInScreenShare", .bool(showInScreenShare))
        put("hooks", .array(hooks.map { .object(["event": .string($0.event), "command": .string($0.command)]) }))
        try obj.encode(to: encoder)
    }

    private static func string(_ v: JSONValue?) -> String? {
        if case .string(let s) = v { return s }
        return nil
    }

    /// Lenient decode: unknown keys are retained in `extra`.
    public init(fromJSONObject obj: [String: JSONValue]) {
        let bool: (String) -> Bool = { key in
            if case .bool(let b) = obj[key] { return b }
            return false
        }
        self.init(
            watchEnabled: bool("watchEnabled"),
            soundCopy: bool("soundCopy"),
            soundPaste: bool("soundPaste"),
            maxItems: {
                if case .number(let n) = obj["maxItems"] { return UInt32(n) }
                return 5000
            }(),
            shortcut: Settings.string(obj["shortcut"]) ?? "CommandOrControl+Shift+V",
            launchAtLogin: bool("launchAtLogin"),
            theme: Settings.string(obj["theme"]) ?? "dark",
            excludedApps: {
                if case .array(let a) = obj["excludedApps"] { return a.compactMap(Settings.string) }
                return []
            }(),
            ignoreTransient: bool("ignoreTransient"),
            ignoreConfidential: bool("ignoreConfidential"),
            generateLinkPreviews: bool("generateLinkPreviews"),
            pasteAsPlainText: bool("pasteAsPlainText"),
            stackDirection: Settings.string(obj["stackDirection"]) ?? "fifo",
            plainTextApps: {
                if case .array(let a) = obj["plainTextApps"] { return a.compactMap(Settings.string) }
                return []
            }(),
            syncDirectory: Settings.string(obj["syncDirectory"]) ?? "",
            showInScreenShare: bool("showInScreenShare"),
            hooks: {
                guard case .array(let a) = obj["hooks"] else { return [] }
                return a.compactMap { v -> Hook? in
                    guard case .object(let o) = v,
                          let e = Settings.string(o["event"]),
                          let cmd = Settings.string(o["command"]) else { return nil }
                    return Hook(event: e, command: cmd)
                }
            }()
        )
        self.extra = obj.filter { !knownSettingsKeys.contains($0.key) }
    }
}
