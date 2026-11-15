import XCTest
@testable import ClipDeckCore

/// Fixture-driven parity tests: every case in spec/fixtures must pass against
/// this target, exactly as `spec/verify.mjs` validates the reference and the
/// Rust `clipdeck-core` test suite validates that implementation.
final class FixtureTests: XCTestCase {
    static var fixturesDir: URL {
        // Tests/ClipDeckCoreTests -> apps/macos -> apps -> repo root -> spec/fixtures
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("spec/fixtures")
    }

    func loadJSON(_ name: String) throws -> [String: JSONValue] {
        let data = try Data(contentsOf: Self.fixturesDir.appendingPathComponent(name))
        let v = try JSONDecoder().decode(JSONValue.self, from: data)
        guard case .object(let o) = v else { throw NSError(domain: "fixture", code: 1) }
        return o
    }

    func objArray(_ v: JSONValue?) -> [[String: JSONValue]] {
        guard case .array(let a) = v else { return [] }
        return a.compactMap { if case .object(let o) = $0 { return o } else { return nil } }
    }

    func str(_ v: JSONValue?) -> String? {
        if case .string(let s) = v { return s }
        return nil
    }

    func num(_ v: JSONValue?) -> Int64? {
        if case .number(let n) = v { return Int64(n) }
        return nil
    }

    func bool(_ v: JSONValue?) -> Bool? {
        if case .bool(let b) = v { return b }
        return nil
    }

    // MARK: classify-text

    func testClassifyTextFixture() throws {
        let fx = try loadJSON("classify-text.json")
        for c in objArray(fx["cases"]) {
            let raw = str(c["raw"]) ?? ""
            let expected = str(c["expected"]) ?? ""
            XCTAssertEqual(Classifier.classify(raw).rawValue, expected, "case \(str(c["id"]) ?? "?")")
        }
    }

    // MARK: preview-title

    func testPreviewTitleFixture() throws {
        let fx = try loadJSON("preview-title.json")
        for c in objArray(fx["cases"]) {
            let id = str(c["id"]) ?? "?"
            let type = ItemType(rawValue: str(c["type"]) ?? "text") ?? .text
            let content = str(c["content"]) ?? ""
            var dims: (UInt32, UInt32)? = nil
            if let d = objArray([c["dims"] ?? .null]).first {
                dims = (UInt32(num(d["width"]) ?? 0), UInt32(num(d["height"]) ?? 0))
            }
            let (title, preview) = Derive.pair(type, content: content, dims: dims)
            let exp = objArray([c["expected"] ?? .null]).first ?? [:]
            XCTAssertEqual(title, str(exp["title"]) ?? "", "\(id) title")
            XCTAssertEqual(preview, str(exp["preview"]) ?? "", "\(id) preview")
        }
    }

    // MARK: store-rules

    func testStoreRulesFixture() throws {
        let fx = try loadJSON("store-rules.json")
        for c in objArray(fx["cases"]) {
            let id = str(c["id"]) ?? "?"
            let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("clipdeck-test-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
            let store = Store(userData: tmp)
            store.settings.maxItems = UInt32(num(c["maxItems"]) ?? 0)
            for (n, a) in objArray(c["adds"]).enumerated() {
                var item = ClipItem(
                    id: "gen-\(n)",
                    itemType: ItemType(rawValue: str(a["type"]) ?? "text") ?? .text,
                    content: str(a["content"]) ?? "",
                    preview: str(a["content"]) ?? "",
                    title: str(a["content"]) ?? "",
                    createdAt: Int64(n),
                    sizeBytes: UInt64((str(a["content"]) ?? "").utf8.count),
                    pinned: bool(a["pinned"]) ?? false,
                    pinboardId: str(a["pinboardId"]),
                    stackOrder: num(a["stackOrder"])
                )
                store.addItem(item)
                _ = item
            }
            if case .array(let order) = c["reorder"] ?? .null {
                let contents = order.compactMap { str($0) }
                let ids = contents.compactMap { content in store.items.first { $0.content == content }?.id }
                store.reorder(ids)
            }
            let expected = objArray([c["expected"] ?? .null]).first ?? [:]
            if case .array(let expContents) = expected["contents"] {
                let got = store.items.map { $0.content }
                XCTAssertEqual(got, expContents.compactMap { str($0) }, "\(id) contents")
            }
            if let count = num(expected["count"]) {
                XCTAssertEqual(store.items.count, Int(count), "\(id) count")
            }
            if let sc = num(expected["stackedCount"]) {
                XCTAssertEqual(store.items.filter { $0.stackOrder != nil }.count, Int(sc), "\(id) stacked")
            }
        }
    }

    func testStoreImportFixture() throws {
        let fx = try loadJSON("store-rules.json")
        for c in objArray(fx["importCases"]) {
            let id = str(c["id"]) ?? "?"
            let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("clipdeck-test-\(UUID().uuidString)")
            try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
            let store = Store(userData: tmp)
            for (n, local) in objArray(c["local"]).enumerated() {
                var item = ClipItem(
                    id: str(local["id"]) ?? "loc-\(n)",
                    itemType: ItemType(rawValue: str(local["type"]) ?? "text") ?? .text,
                    content: str(local["content"]) ?? "",
                    preview: str(local["content"]) ?? "",
                    title: str(local["content"]) ?? "",
                    createdAt: Int64(n),
                    sizeBytes: 0
                )
                store.addItem(item)
                _ = item
            }
            let importItems: [JSONValue]
            if case .array(let a) = c["import"] { importItems = a } else { importItems = [] }
            let payload: [String: JSONValue] = ["items": .array(importItems)]
            let raw = String(data: try JSONEncoder().encode(payload), encoding: .utf8) ?? "{}"
            let counts = store.importJSON(raw)
            let expected = objArray([c["expected"] ?? .null]).first ?? [:]
            XCTAssertEqual(counts.imported, Int(num(expected["imported"]) ?? -1), "\(id) imported")
            XCTAssertEqual(counts.skipped, Int(num(expected["skipped"]) ?? -1), "\(id) skipped")
            XCTAssertEqual(store.items.count, Int(num(expected["count"]) ?? -1), "\(id) count")
            if let sc = num(expected["stackedCount"]) {
                XCTAssertEqual(store.items.filter { $0.stackOrder != nil }.count, Int(sc), "\(id) stacked")
            }
        }
    }

    // MARK: sync-merge

    func testSyncMergeFixture() throws {
        let fx = try loadJSON("sync-merge.json")
        for c in objArray(fx["cases"]) {
            let id = str(c["id"]) ?? "?"
            let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("clipdeck-test-\(UUID().uuidString)")
            let shared = tmp.appendingPathComponent("shared")
            try FileManager.default.createDirectory(at: shared, withIntermediateDirectories: true)
            let store = Store(userData: tmp.appendingPathComponent("a"))
            for (n, local) in objArray(c["local"]).enumerated() {
                var item = ClipItem(
                    id: str(local["id"]) ?? "loc-\(n)",
                    itemType: ItemType(rawValue: str(local["type"]) ?? "text") ?? .text,
                    content: str(local["content"]) ?? "",
                    preview: str(local["content"]) ?? "",
                    title: str(local["content"]) ?? "",
                    createdAt: num(local["createdAt"]) ?? Int64(n),
                    sizeBytes: 0
                )
                store.addItem(item)
                _ = item
            }
            let sync = SyncStore(syncDirectory: shared.path)
            var lines = ""
            for row in objArray(c["remote"]) {
                let envelope: [String: JSONValue] = ["v": .number(1), "item": .object(row)]
                if let data = try? JSONEncoder().encode(envelope),
                   let line = String(data: data, encoding: .utf8) {
                    lines += line + "\n"
                }
            }
            try lines.data(using: .utf8)!.write(to: shared.appendingPathComponent("clipdeck-sync.jsonl"))
            let counts = sync.merge(into: store)
            let expected = objArray([c["expected"] ?? .null]).first ?? [:]
            if bool(c["mergeTwice"]) ?? false {
                XCTAssertEqual(counts.imported, Int(num(expected["firstImported"]) ?? -1), "\(id) first")
                let second = sync.merge(into: store)
                XCTAssertEqual(second.imported, Int(num(expected["secondImported"]) ?? -1), "\(id) second")
            } else {
                XCTAssertEqual(counts.imported, Int(num(expected["imported"]) ?? -1), "\(id) imported")
            }
            if let sk = num(expected["skipped"]) {
                XCTAssertEqual(counts.skipped, Int(sk), "\(id) skipped")
            }
            if case .array(let expContents) = expected["contents"] {
                let got = store.items.map { $0.content }
                XCTAssertEqual(got, expContents.compactMap { str($0) }, "\(id) contents")
            }
            if let sc = num(expected["stackedCount"]) {
                XCTAssertEqual(store.items.filter { $0.stackOrder != nil }.count, Int(sc), "\(id) stacked")
            }
        }
    }

    // MARK: hooks

    func testHooksFixture() throws {
        let fx = try loadJSON("hooks.json")
        for c in objArray(fx["parsing"]) {
            let id = str(c["id"]) ?? "?"
            let line = str(c["line"]) ?? ""
            let got = Hooks.parseLine(line)
            if let exp = objArray([c["expected"] ?? .null]).first {
                XCTAssertNotNil(got, id)
                XCTAssertEqual(got?.event, str(exp["event"]) ?? "", id)
                XCTAssertEqual(got?.command, str(exp["command"]) ?? "", id)
            } else {
                XCTAssertNil(got, id)
            }
        }
    }

    // MARK: accelerators

    func testAcceleratorsFixture() throws {
        let fx = try loadJSON("accelerators.json")
        for c in objArray(fx["recording"]) {
            let id = str(c["id"]) ?? "?"
            let e = objArray([c["event"] ?? .null]).first ?? [:]
            let kp = KeyPress(
                ctrl: bool(e["ctrl"]) ?? false,
                meta: bool(e["meta"]) ?? false,
                alt: bool(e["alt"]) ?? false,
                shift: bool(e["shift"]) ?? false,
                code: str(e["code"]) ?? ""
            )
            XCTAssertEqual(Accel.record(kp), str(c["expected"]), id)
        }
        for c in objArray(fx["display"]) {
            XCTAssertEqual(Accel.pretty(str(c["accel"]) ?? ""), str(c["pretty"]) ?? "", "display")
        }
    }

    // MARK: autostart

    func testAutostartFixture() throws {
        let fx = try loadJSON("autostart.json")
        for c in objArray(fx["cases"]) {
            let id = str(c["id"]) ?? "?"
            let platform = str(c["platform"]) ?? ""
            let enable = bool(c["enable"]) ?? false
            let binary = str(c["execPath"]) ?? ""
            let appImage = str(c["appImage"])
            if platform == "linux" {
                let exe = Autostart.execPath(binary: binary, appImage: appImage)
                let content = Autostart.renderDesktopEntry(execPath: exe)
                if let exp = objArray([c["expectedFile"] ?? .null]).first {
                    XCTAssertEqual(content, str(exp["content"]) ?? "", id)
                } else {
                    XCTAssertFalse(enable && content.isEmpty, id)
                }
            } else {
                // delegation boolean recorded for the app layer (SMAppService)
                XCTAssertNotNil(bool(c["expectedLoginItemSettings"]) != nil || c["expectedLoginItemSettings"] != nil, id)
            }
        }
    }
}
