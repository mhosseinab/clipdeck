import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Store } from "../electron/store.ts";
import { SyncStore } from "../electron/sync.ts";
import { runHooks } from "../electron/hooks.ts";

function tmp(): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), "clipdeck-test-"));
}

function makeItem(content: string, over: Record<string, unknown> = {}) {
  return {
    id: crypto.randomUUID(),
    type: "text" as const,
    content,
    preview: content.slice(0, 20),
    title: content.slice(0, 20),
    sourceApp: null,
    createdAt: Date.now(),
    sizeBytes: content.length,
    pinned: false,
    pinboardId: null,
    stackOrder: null,
    ...over,
  };
}

test("addItem deduplicates identical content and moves it to the front", () => {
  const store = new Store(tmp());
  const a = store.addItem(makeItem("hello"));
  store.addItem(makeItem("world"));
  const again = store.addItem(makeItem("hello"));
  assert.equal(again.id, a.id, "duplicate keeps original id");
  assert.equal(store.items().length, 2);
  assert.equal(store.items()[0].content, "hello");
});

test("export/import round-trips items and skips duplicates", () => {
  const dir = tmp();
  const store = new Store(dir);
  store.addItem(makeItem("alpha"));
  store.addItem(makeItem("beta"));
  const exported = store.exportJSON();
  assert.match(exported, /"alpha"/);

  const store2 = new Store(dir + "/other");
  const r1 = store2.importJSON(exported);
  assert.equal(r1.imported, 2);
  const r2 = store2.importJSON(exported);
  assert.equal(r2.imported, 0);
  assert.equal(r2.skipped, 2);
  assert.equal(store2.items().length, 2);
});

test("import rejects malformed payloads without crashing", () => {
  const store = new Store(tmp());
  const r = store.importJSON(JSON.stringify({ items: [null, { content: 5 }, makeItem("ok")] }));
  assert.equal(r.imported, 1);
  assert.equal(r.skipped, 2);
});

test("trim keeps pinned items beyond the cap", () => {
  const store = new Store(tmp());
  store.setSettings({ maxItems: 2 });
  store.addItem(makeItem("pinned-1", { pinned: true }));
  store.addItem(makeItem("pinned-2", { pinned: true }));
  store.addItem(makeItem("a"));
  store.addItem(makeItem("b"));
  store.addItem(makeItem("c"));
  const contents = store.items().map((i) => i.content);
  assert.ok(contents.includes("pinned-1"));
  assert.ok(contents.includes("pinned-2"));
  assert.ok(!contents.includes("a"), "oldest unpinned trimmed");
});

test("stackOrder changes do not dedupe", () => {
  const store = new Store(tmp());
  const a = store.addItem(makeItem("queued"));
  store.update(a.id, { stackOrder: 1 });
  const b = store.addItem(makeItem("queued"));
  assert.notEqual(a.id, b.id);
  assert.equal(store.items().filter((i) => i.stackOrder !== null).length, 1);
});

test("SyncStore appends and merges items from other machines", () => {
  const dir = tmp();
  const shared = path.join(dir, "shared");
  const store = new Store(path.join(dir, "a"));
  store.setSettings({ syncDirectory: shared });
  const sync = new SyncStore(store);
  assert.ok(sync.enabled());

  // real flow: watcher records locally, then appends to sync
  store.addItem(makeItem("from-machine-a"));
  sync.append(makeItem("from-machine-a"));
  // simulate machine B appending to the same file
  const file = path.join(shared, "clipdeck-sync.jsonl");
  fs.appendFileSync(file, JSON.stringify({ v: 1, item: makeItem("from-machine-b") }) + "\n");

  const merged = sync.merge();
  assert.equal(merged, 1, "only the remote item is merged");
  assert.deepEqual(
    store.items().map((i) => i.content).sort(),
    ["from-machine-a", "from-machine-b"],
  );
  assert.equal(sync.merge(), 0, "second merge is a no-op");
});

test("runHooks executes copy hooks with stdin and never throws", () => {
  const marker = path.join(tmp(), "out.txt");
  const settings = {
    watchEnabled: true, soundCopy: true, soundPaste: true, maxItems: 10, shortcut: "x",
    launchAtLogin: false, theme: "dark" as const, excludedApps: [], ignoreTransient: true,
    ignoreConfidential: true, generateLinkPreviews: true, pasteAsPlainText: false,
    stackDirection: "fifo" as const, plainTextApps: [], syncDirectory: "",
    showInScreenShare: true,
    hooks: [{ event: "copy" as const, command: `cat > ${marker}` }],
  };
  runHooks("copy", makeItem("hook-payload"), settings);
  const deadline = Date.now() + 4000;
  while (Date.now() < deadline) {
    if (fs.existsSync(marker) && fs.readFileSync(marker, "utf8") === "hook-payload") break;
  }
  assert.equal(fs.readFileSync(marker, "utf8"), "hook-payload");
  runHooks("paste", makeItem("ignored"), settings); // wrong event: no throw
});
