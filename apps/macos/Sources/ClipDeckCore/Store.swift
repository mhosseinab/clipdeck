import Foundation

public struct ImportCounts: Equatable, Sendable {
    public var imported: Int
    public var skipped: Int
}

public enum StackDir { case fifo, lifo }

/// Store per spec/behavior.md §2.4, §5, §6. JSON-compatible with the v1 format.
public final class Store {
    public private(set) var items: [ClipItem]
    public private(set) var pinboards: [Pinboard]
    public var settings: Settings
    public let imageDir: URL
    private let file: URL

    public init(userData: URL) {
        file = userData.appendingPathComponent("clipdeck-store.json")
        imageDir = userData.appendingPathComponent("images")
        try? FileManager.default.createDirectory(at: imageDir, withIntermediateDirectories: true)
        if let data = try? Data(contentsOf: file),
           let sf = try? JSONDecoder().decode(StoreFile.self, from: data) {
            items = sf.items
            pinboards = sf.pinboards
            settings = sf.settings
        } else {
            items = []
            pinboards = []
            settings = .default
        }
    }

    public func save() {
        let sf = StoreFile(items: items, pinboards: pinboards, settings: settings)
        if let data = try? JSONEncoder().encode(sf) {
            try? data.write(to: file, options: .atomic)
        }
    }

    /* ---------- capture pipeline (§2.4) ---------- */

    /// Dedupe-refresh or insert + trim. Returns the captured item.
    @discardableResult
    public func addItem(_ item: ClipItem) -> ClipItem {
        var item = item
        if item.stackOrder == nil,
           let idx = items.firstIndex(where: {
               $0.isDedupeCandidate && $0.itemType == item.itemType && $0.content == item.content
           }) {
            var existing = items.remove(at: idx)
            existing.createdAt = item.createdAt
            existing.sourceApp = item.sourceApp
            items.insert(existing, at: 0)
            trim()
            return items[0]
        }
        items.insert(item, at: 0)
        trim()
        return item
    }

    /// §5.2: kept = keep (list order) ++ overflow[0 .. maxItems - |keep|].
    private func trim() {
        let max = Int(settings.maxItems)
        var keep: [ClipItem] = []
        var overflow: [ClipItem] = []
        for i in items {
            if i.isKeep { keep.append(i) } else { overflow.append(i) }
        }
        let budget = max - keep.count
        keep.append(contentsOf: overflow.prefix(Swift.max(0, budget)))
        items = keep
    }

    public func find(_ id: String) -> ClipItem? { items.first { $0.id == id } }

    @discardableResult
    public func update(_ id: String, _ mutate: (inout ClipItem) -> Void) -> ClipItem? {
        guard let idx = items.firstIndex(where: { $0.id == id }) else { return nil }
        mutate(&items[idx])
        return items[idx]
    }

    public func remove(_ id: String) { items.removeAll { $0.id == id } }

    public func clearHistory() { items = items.filter { $0.isKeep } }

    /// §5.1: listed ids first in given order; unlisted keep relative order after.
    public func reorder(_ ids: [String]) {
        var placed = Set<String>()
        var ordered: [ClipItem] = []
        for id in ids {
            if let idx = items.firstIndex(where: { $0.id == id && !placed.contains($0.id) }) {
                placed.insert(items[idx].id)
                ordered.append(items.remove(at: idx))
            }
        }
        ordered.append(contentsOf: items)
        items = ordered
    }

    /* ---------- stack (§6) ---------- */

    @discardableResult
    public func pushStack(_ id: String) -> ClipItem? {
        let len = Int64(items.filter { $0.stackOrder != nil }.count)
        return update(id) { $0.stackOrder = len + 1 }
    }

    @discardableResult
    public func popStack(_ dir: StackDir) -> ClipItem? {
        let queued = items.enumerated().filter { $0.element.stackOrder != nil }
        let pick: (Int, ClipItem)? = queued.min { a, b in
            let (oa, ob) = (a.element.stackOrder ?? 0, b.element.stackOrder ?? 0)
            switch dir {
            case .fifo: return oa == ob ? a.offset < b.offset : oa < ob
            case .lifo: return oa == ob ? a.offset < b.offset : oa > ob
            }
        }
        guard let pick else { return nil }
        return items.remove(at: pick.offset)
    }

    @discardableResult
    public func unstack(_ id: String) -> ClipItem? {
        update(id) { $0.stackOrder = nil }
    }

    public func clearStack() {
        for i in items.indices { items[i].stackOrder = nil }
    }

    public var stackItems: [ClipItem] {
        items.filter { $0.stackOrder != nil }.sorted { ($0.stackOrder ?? 0) < ($1.stackOrder ?? 0) }
    }

    /* ---------- pinboards ---------- */

    @discardableResult
    public func addPinboard(_ name: String) -> Pinboard {
        let pb = Pinboard(id: UUID().uuidString.lowercased(), name: name, icon: "star", position: UInt32(pinboards.count))
        pinboards.append(pb)
        return pb
    }

    public func removePinboard(_ id: String) {
        pinboards.removeAll { $0.id == id }
        for i in items.indices where items[i].pinboardId == id {
            items[i].pinboardId = nil
        }
    }

    /* ---------- export / import (§5.3) ---------- */

    public func exportJSON() -> String {
        let iso = ISO8601DateFormatter().string(from: Date())
        let payload: [String: JSONValue] = [
            "version": .number(1),
            "exportedAt": .string(iso),
            "items": .array(items.map { Self.jsonValue($0) }),
            "pinboards": .array(pinboards.map {
                .object(["id": .string($0.id), "name": .string($0.name), "icon": .string($0.icon), "position": .number(Double($0.position))])
            }),
        ]
        let data = (try? JSONEncoder().encode(payload)) ?? Data()
        return String(data: data, encoding: .utf8) ?? "{}"
    }

    public func importJSON(_ raw: String) -> ImportCounts {
        guard let data = raw.data(using: .utf8),
              let parsed = try? JSONDecoder().decode(JSONValue.self, from: data),
              case .object(let obj) = parsed else {
            return ImportCounts(imported: 0, skipped: 1)
        }
        var imported = 0
        var skipped = 0
        if case .array(let incoming) = obj["items"] {
            for rawItem in incoming {
                guard let item = Self.item(fromJSON: rawItem) else {
                    skipped += 1
                    continue
                }
                if items.contains(where: { $0.dedupeKey == item.dedupeKey || $0.id == item.id }) {
                    skipped += 1
                    continue
                }
                var item = item
                item.stackOrder = nil
                items.append(item)
                imported += 1
            }
        }
        if case .array(let pbs) = obj["pinboards"] {
            for pbJSON in pbs {
                if case .object(let o) = pbJSON,
                   case .string(let id) = o["id"], !id.isEmpty,
                   case .string(let name) = o["name"] {
                    let icon: String
                    if case .string(let i) = o["icon"] { icon = i } else { icon = "star" }
                    var pos: UInt32 = UInt32(pinboards.count)
                    if case .number(let n) = o["position"] { pos = UInt32(n) }
                    if !pinboards.contains(where: { $0.id == id }) {
                        pinboards.append(Pinboard(id: id, name: name, icon: icon, position: pos))
                    }
                }
            }
        }
        if imported > 0 {
            sortNewestFirst()
            retrim()
        }
        return ImportCounts(imported: imported, skipped: skipped)
    }

    /// Re-apply the §5.2 trim after bulk mutations.
    public func retrim() { trim() }

    public func sortNewestFirst() {
        items.sort {
            $0.createdAt == $1.createdAt ? $0.id < $1.id : $0.createdAt > $1.createdAt
        }
    }

    /* ---------- JSON bridging ---------- */

    /// Lenient parser for import/sync rows (§5.3, §9): requires non-empty id,
    /// string content, valid type; derives missing preview/title.
    public static func item(fromJSON v: JSONValue) -> ClipItem? {
        guard case .object(let o) = v,
              case .string(let id) = o["id"], !id.isEmpty,
              case .string(let content) = o["content"],
              case .string(let typeRaw) = o["type"],
              let type = ItemType(rawValue: typeRaw) else { return nil }
        let title: String
        let preview: String
        if case .string(let t) = o["title"], !t.isEmpty {
            title = t
            if case .string(let p) = o["preview"] { preview = p } else { preview = t }
        } else {
            (title, preview) = Derive.pair(type, content: content, dims: nil)
        }
        var createdAt: Int64 = 0
        if case .number(let n) = o["createdAt"] { createdAt = Int64(n) }
        var sizeBytes: UInt64 = 0
        if case .number(let n) = o["sizeBytes"] { sizeBytes = UInt64(n) }
        var pinned = false
        if case .bool(let b) = o["pinned"] { pinned = b }
        var sourceApp: String?
        if case .string(let s) = o["sourceApp"] { sourceApp = s }
        var pinboardId: String?
        if case .string(let s) = o["pinboardId"] { pinboardId = s }
        var imageFile: String?
        if case .string(let s) = o["imageFile"] { imageFile = s }
        return ClipItem(id: id, itemType: type, content: content, preview: preview, title: title,
                        sourceApp: sourceApp, createdAt: createdAt, sizeBytes: sizeBytes,
                        pinned: pinned, pinboardId: pinboardId, stackOrder: nil, imageFile: imageFile)
    }

    public static func jsonValue(_ item: ClipItem) -> JSONValue {
        var o: [String: JSONValue] = [
            "id": .string(item.id),
            "type": .string(item.itemType.rawValue),
            "content": .string(item.content),
            "preview": .string(item.preview),
            "title": .string(item.title),
            "createdAt": .number(Double(item.createdAt)),
            "sizeBytes": .number(Double(item.sizeBytes)),
            "pinned": .bool(item.pinned),
            "pinboardId": item.pinboardId.map { .string($0) } ?? .null,
            "stackOrder": item.stackOrder.map { .number(Double($0)) } ?? .null,
        ]
        if let app = item.sourceApp { o["sourceApp"] = .string(app) }
        if let img = item.imageFile { o["imageFile"] = .string(img) }
        return .object(o)
    }
}
