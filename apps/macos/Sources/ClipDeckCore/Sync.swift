import Foundation

/// Folder-based sync per spec/behavior.md §9.
public final class SyncStore {
    private var file: URL?
    private var lastSize: UInt64 = 0

    public init(syncDirectory: String) {
        let dir = syncDirectory.trimmingCharacters(in: .whitespaces)
        if dir.isEmpty {
            file = nil
        } else {
            let url = URL(fileURLWithPath: dir)
            try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            file = url.appendingPathComponent("clipdeck-sync.jsonl")
            if let attrs = try? FileManager.default.attributesOfItem(atPath: url.appendingPathComponent("clipdeck-sync.jsonl").path),
               let size = attrs[.size] as? UInt64 {
                lastSize = size
            }
        }
    }

    public var enabled: Bool { file != nil }

    /// Append a newly captured item (post-dedupe).
    public func append(_ item: ClipItem) {
        guard let file else { return }
        let row: [String: JSONValue] = ["v": .number(1), "item": Store.jsonValue(item)]
        guard let data = try? JSONEncoder().encode(row),
              let line = String(data: data, encoding: .utf8) else { return }
        if let handle = try? FileHandle(forWritingTo: file) {
            defer { try? handle.close() }
            try? handle.seekToEnd()
            try? handle.write(contentsOf: Data((line + "\n").utf8))
        } else {
            try? (line + "\n").data(using: .utf8)?.write(to: file)
        }
    }

    /// Merge lines added since the last merge (complete lines only).
    @discardableResult
    public func merge(into store: Store) -> ImportCounts {
        guard let file else { return ImportCounts(imported: 0, skipped: 0) }
        guard let attrs = try? FileManager.default.attributesOfItem(atPath: file.path),
              let size = attrs[.size] as? UInt64, size > lastSize,
              let all = try? Data(contentsOf: file) else {
            return ImportCounts(imported: 0, skipped: 0)
        }
        let newData = all.dropFirst(Int(lastSize))
        guard let lastNL = newData.lastIndex(of: UInt8(ascii: "\n")) else {
            return ImportCounts(imported: 0, skipped: 0)
        }
        let consumed = newData.distance(from: newData.startIndex, to: lastNL) + 1
        lastSize += UInt64(consumed)
        let chunk = String(decoding: newData.prefix(consumed), as: UTF8.self)

        var imported = 0
        var skipped = 0
        var any = false
        for line in chunk.split(separator: "\n") {
            if line.trimmingCharacters(in: .whitespaces).isEmpty { continue }
            guard let data = String(line).data(using: .utf8),
                  let row = try? JSONDecoder().decode(JSONValue.self, from: data),
                  case .object(let o) = row else {
                skipped += 1
                continue
            }
            let itemJSON = o["item"] ?? .null
            guard let item = Store.item(fromJSON: itemJSON) else {
                skipped += 1
                continue
            }
            if store.items.contains(where: { $0.dedupeKey == item.dedupeKey || $0.id == item.id }) {
                skipped += 1
                continue
            }
            var item = item
            item.stackOrder = nil
            store.items.append(item)
            imported += 1
            any = true
        }
        if any {
            store.sortNewestFirst()
        }
        return ImportCounts(imported: imported, skipped: skipped)
    }
}
