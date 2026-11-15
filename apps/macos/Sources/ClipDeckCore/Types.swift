import Foundation

public enum ItemType: String, Codable, Sendable {
    case text, link, code, color, image, file, html
}

public struct ClipItem: Codable, Equatable, Sendable {
    public var id: String
    public var itemType: ItemType
    public var content: String
    public var preview: String
    public var title: String
    public var sourceApp: String?
    public var createdAt: Int64
    public var sizeBytes: UInt64
    public var pinned: Bool
    public var pinboardId: String?
    public var stackOrder: Int64?
    public var imageFile: String?

    enum CodingKeys: String, CodingKey {
        case id, content, preview, title, pinned
        case itemType = "type"
        case sourceApp, createdAt, sizeBytes, pinboardId, stackOrder, imageFile
    }

    public init(id: String, itemType: ItemType, content: String, preview: String, title: String,
                sourceApp: String? = nil, createdAt: Int64, sizeBytes: UInt64,
                pinned: Bool = false, pinboardId: String? = nil,
                stackOrder: Int64? = nil, imageFile: String? = nil) {
        self.id = id
        self.itemType = itemType
        self.content = content
        self.preview = preview
        self.title = title
        self.sourceApp = sourceApp
        self.createdAt = createdAt
        self.sizeBytes = sizeBytes
        self.pinned = pinned
        self.pinboardId = pinboardId
        self.stackOrder = stackOrder
        self.imageFile = imageFile
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        itemType = try c.decode(ItemType.self, forKey: .itemType)
        content = try c.decode(String.self, forKey: .content)
        preview = try c.decodeIfPresent(String.self, forKey: .preview) ?? ""
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        sourceApp = try c.decodeIfPresent(String.self, forKey: .sourceApp)
        createdAt = try c.decodeIfPresent(Int64.self, forKey: .createdAt) ?? 0
        sizeBytes = try c.decodeIfPresent(UInt64.self, forKey: .sizeBytes) ?? 0
        pinned = try c.decodeIfPresent(Bool.self, forKey: .pinned) ?? false
        pinboardId = try c.decodeIfPresent(String.self, forKey: .pinboardId)
        stackOrder = try c.decodeIfPresent(Int64.self, forKey: .stackOrder)
        imageFile = try c.decodeIfPresent(String.self, forKey: .imageFile)
    }

    /// Dedupe key per behavior.md §2.4.
    public var dedupeKey: String { "\(itemType.rawValue)|\(content)" }
    /// §5.2 keep set.
    public var isKeep: Bool { pinned || pinboardId != nil || stackOrder != nil }
    public var isDedupeCandidate: Bool { stackOrder == nil }
}

public struct Pinboard: Codable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var icon: String
    public var position: UInt32
}

public struct Hook: Codable, Equatable, Sendable {
    public var event: String
    public var command: String
}

public struct Settings: Equatable, Sendable {
    public var watchEnabled: Bool
    public var soundCopy: Bool
    public var soundPaste: Bool
    public var maxItems: UInt32
    public var shortcut: String
    public var launchAtLogin: Bool
    public var theme: String
    public var excludedApps: [String]
    public var ignoreTransient: Bool
    public var ignoreConfidential: Bool
    public var generateLinkPreviews: Bool
    public var pasteAsPlainText: Bool
    public var stackDirection: String
    public var plainTextApps: [String]
    public var syncDirectory: String
    public var showInScreenShare: Bool
    public var hooks: [Hook]
    /// Unknown keys read from disk are preserved on write (behavior.md §1.3).
    public var extra: [String: JSONValue] = [:]

    public static let `default` = Settings(
        watchEnabled: true, soundCopy: true, soundPaste: true, maxItems: 5000,
        shortcut: "CommandOrControl+Shift+V", launchAtLogin: true, theme: "dark",
        excludedApps: ["com.apple.keychainaccess", "com.apple.Passwords", "1Password", "Bitwarden", "KeePass"],
        ignoreTransient: true, ignoreConfidential: true, generateLinkPreviews: true,
        pasteAsPlainText: false, stackDirection: "fifo", plainTextApps: [],
        syncDirectory: "", showInScreenShare: true, hooks: []
    )
}

public struct StoreFile: Codable, Sendable {
    public var items: [ClipItem]
    public var pinboards: [Pinboard]
    public var settings: Settings
}

public let ellipsis = "\u{2026}"
