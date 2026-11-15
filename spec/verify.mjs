#!/usr/bin/env node
/**
 * spec/verify.mjs — fixture self-check with a REFERENCE IMPLEMENTATION of
 * spec/behavior.md (§3, §4, §5, §8, §9, §10, §11). This is a test oracle only;
 * it is not shipped. The Rust and Swift implementations must pass the same
 * fixture files.
 *
 * Usage: node spec/verify.mjs   (exit 0 = all fixtures internally consistent)
 */
import { readFileSync, readdirSync } from "node:fs";
import { join, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";

const SPEC = join(dirname(fileURLToPath(import.meta.url)));
const load = (p) => JSON.parse(readFileSync(join(SPEC, p), "utf8"));

let failures = 0;
let checks = 0;
function check(ok, msg) {
  checks++;
  if (!ok) {
    failures++;
    console.error("FAIL:", msg);
  }
}

/* ---------- reference: §3 classification ---------- */
function classify(raw) {
  const t = raw.trim();
  if (/^#[0-9a-fA-F]{3,8}$/.test(t) || /^rgba?\(/.test(t)) return "color";
  if (/^(https?:\/\/|www\.)\S+$/.test(t)) return "link";
  const score =
    (/[;{}()[\]]/.test(t) ? 1 : 0) +
    (/(const |let |var |function |def |class |import |#include|<\/|\/>)/.test(t) ? 1 : 0) +
    (/\n/.test(t) && / {2,}|\t/.test(raw) ? 1 : 0);
  return score >= 2 ? "code" : "text";
}

/* ---------- reference: §4 preview/title (Unicode-scalar truncation) ---------- */
const ELLIPSIS = "\u2026";
const scalars = (s) => Array.from(s);
function truncate(s, cap) {
  const a = scalars(s);
  return a.length > cap ? a.slice(0, cap).join("") + ELLIPSIS : s;
}
function derive(type, content, dims) {
  if (type === "link") {
    let host;
    try {
      host = new URL(content).hostname.replace(/^www\./, "");
    } catch {
      host = truncate(content, 80);
    }
    return { title: host, preview: host };
  }
  if (type === "color") {
    const t = content.trim();
    return { title: t, preview: t };
  }
  if (type === "image") {
    return { title: "Image", preview: `${dims.width}\u00d7${dims.height} image` };
  }
  if (type === "file") {
    const lines = content.split("\n");
    const label = lines.length === 1 ? lines[0].split("/").pop() : `${lines.length} files`;
    return { title: label, preview: label };
  }
  const first = content.split("\n")[0].trim();
  return { title: truncate(first, 60) || type, preview: truncate(first, 120) };
}

/* ---------- reference: §5 store (add/dedupe/trim/reorder/import) ---------- */
function simulateStore(caseObj) {
  const maxItems = caseObj.maxItems;
  let items = [];
  let nextId = 0;
  const keyOf = (i) => i.type + "|" + i.content;
  for (const a of caseObj.adds) {
    const item = {
      id: a.id ?? `gen-${nextId++}`,
      type: a.type,
      content: a.content,
      createdAt: nextId,
      pinned: !!a.pinned,
      pinboardId: a.pinboardId ?? null,
      stackOrder: a.stackOrder ?? null,
    };
    const dupIdx = items.findIndex(
      (i) => i.type === item.type && i.content === item.content && i.stackOrder === null,
    );
    if (dupIdx >= 0) {
      const [existing] = items.splice(dupIdx, 1);
      items.unshift(existing);
    } else {
      items.unshift(item);
    }
    // trim
    const keep = items.filter((i) => i.pinned || i.pinboardId !== null || i.stackOrder !== null);
    const overflow = items.filter((i) => !(i.pinned || i.pinboardId !== null || i.stackOrder !== null));
    items = keep.concat(overflow.slice(0, Math.max(0, maxItems - keep.length)));
  }
  if (caseObj.reorder) {
    const byContent = new Map(items.map((i) => [i.content, i]));
    const ordered = [];
    for (const c of caseObj.reorder) {
      const it = byContent.get(c);
      if (it) {
        ordered.push(it);
        byContent.delete(c);
      }
    }
    items = ordered.concat([...byContent.values()]);
  }
  return items;
}

function simulateImport(local, incoming) {
  let items = local.map((l, n) => ({
    id: l.id ?? `loc-${n}`,
    type: l.type,
    content: l.content,
    stackOrder: null,
  }));
  let imported = 0;
  let skipped = 0;
  for (const raw of incoming) {
    const valid =
      raw && typeof raw.id === "string" && raw.id.length > 0 && typeof raw.content === "string";
    if (!valid) {
      skipped++;
      continue;
    }
    const key = raw.type + "|" + raw.content;
    const keySeen = items.some((i) => i.type + "|" + i.content === key);
    const idSeen = items.some((i) => i.id === raw.id);
    if (keySeen || idSeen) {
      skipped++;
      continue;
    }
    items.push({ id: raw.id, type: raw.type, content: raw.content, stackOrder: null });
    imported++;
  }
  return { imported, skipped, items };
}

/* ---------- reference: §9 sync merge ---------- */
function simulateSync(local, remote) {
  let items = local.map((l) => ({ ...l, stackOrder: null }));
  let imported = 0;
  let skipped = 0;
  for (const row of remote) {
    const it = row && row.item ? row.item : row;
    const valid = it && typeof it.id === "string" && it.id.length > 0 && typeof it.content === "string" && typeof it.type === "string";
    if (!valid) {
      skipped++;
      continue;
    }
    const key = it.type + "|" + it.content;
    if (items.some((i) => i.type + "|" + i.content === key || i.id === it.id)) {
      skipped++;
      continue;
    }
    items.push({ id: it.id, type: it.type, content: it.content, createdAt: it.createdAt, stackOrder: null });
    imported++;
  }
  if (imported > 0) {
    items.sort((a, b) => (b.createdAt ?? 0) - (a.createdAt ?? 0) || (a.id < b.id ? -1 : 1));
  }
  return { imported, skipped, items };
}

/* ---------- reference: §8 hooks parsing ---------- */
function parseHookLine(line) {
  const eq = line.indexOf("=");
  if (eq <= 0) return null;
  const event = line.slice(0, eq).trim();
  const command = line.slice(eq + 1).trim();
  if ((event === "copy" || event === "paste") && command) return { event, command };
  return null;
}

/* ---------- reference: §10 accelerators ---------- */
const CODE_MAP = {
  Space: "Space", Enter: "Enter", Escape: "Escape", Tab: "Tab",
  Backspace: "Backspace", Delete: "Delete",
  ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
  Comma: "Comma", Period: "Period", Slash: "Slash", Backslash: "Backslash",
  Minus: "Minus", Equal: "Equal", Semicolon: "Semicolon", Quote: "Quote",
  Backquote: "Backquote", BracketLeft: "BracketLeft", BracketRight: "BracketRight",
};
function recordAccel(ev, escapeCancels = true) {
  if (escapeCancels && ev.code === "Escape") return null;
  const parts = [];
  if (ev.ctrl || ev.meta) parts.push("CommandOrControl");
  if (ev.alt) parts.push("Alt");
  if (ev.shift) parts.push("Shift");
  let key = null;
  const letter = /^Key([A-Z])$/.exec(ev.code);
  const digit = /^Digit(\d)$/.exec(ev.code);
  const fn = /^F(\d{1,2})$/.exec(ev.code);
  if (letter) key = letter[1];
  else if (digit) key = digit[1];
  else if (fn && Number(fn[1]) >= 1 && Number(fn[1]) <= 24) key = "F" + fn[1];
  else key = CODE_MAP[ev.code] ?? null;
  if (!key) return null;
  parts.push(key);
  return parts.join("+");
}
const PRETTY = {
  CommandOrControl: "\u2318/Ctrl", Command: "\u2318", Control: "Ctrl",
  Shift: "\u21e7", Alt: "\u2325",
};
const prettyAccel = (a) => a.split("+").map((p) => PRETTY[p] ?? p).join("+");

/* ---------- reference: §11 autostart ---------- */
function renderDesktopEntry(execPath) {
  return [
    "[Desktop Entry]",
    "Type=Application",
    "Name=ClipDeck",
    "Comment=Clipboard manager",
    `Exec="${execPath}"`,
    "X-GNOME-Autostart-enabled=true",
    "Hidden=false",
    "NoDisplay=false",
    "",
  ].join("\n");
}

/* ================= validation ================= */

const files = readdirSync(join(SPEC, "fixtures")).filter((f) => f.endsWith(".json"));
const fixtures = Object.fromEntries(files.map((f) => [f.replace(".json", ""), load("fixtures/" + f)]));
check(new Set(files.map((f) => f.replace(".json", ""))).size === files.length, "fixture names unique");

for (const [name, fx] of Object.entries(fixtures)) {
  const ids = (fx.cases ?? fx.recording ?? fx.parsing ?? []).map((c) => c.id);
  check(new Set(ids).size === ids.length, `${name}: case ids unique`);
}

// ---- classify-text
for (const c of fixtures["classify-text"].cases) {
  check(classify(c.raw) === c.expected, `classify-text/${c.id}: got ${classify(c.raw)}, want ${c.expected}`);
}

// ---- preview-title
for (const c of fixtures["preview-title"].cases) {
  const got = derive(c.type, c.content, c.dims ?? { width: 1, height: 1 });
  check(got.title === c.expected.title, `preview-title/${c.id}: title got ${JSON.stringify(got.title)}, want ${JSON.stringify(c.expected.title)}`);
  check(got.preview === c.expected.preview, `preview-title/${c.id}: preview got ${JSON.stringify(got.preview)}, want ${JSON.stringify(c.expected.preview)}`);
}

// ---- store-rules
for (const c of fixtures["store-rules"].cases) {
  const items = simulateStore(c);
  const contents = items.map((i) => i.content);
  check(JSON.stringify(contents) === JSON.stringify(c.expected.contents), `store-rules/${c.id}: contents got ${JSON.stringify(contents)}, want ${JSON.stringify(c.expected.contents)}`);
  if (c.expected.count !== undefined) check(items.length === c.expected.count, `store-rules/${c.id}: count`);
  if (c.expected.stackedCount !== undefined) {
    check(items.filter((i) => i.stackOrder !== null).length === c.expected.stackedCount, `store-rules/${c.id}: stackedCount`);
  }
}
for (const c of fixtures["store-rules"].importCases) {
  const r = simulateImport(c.local, c.import);
  check(r.imported === c.expected.imported, `store-rules/${c.id}: imported ${r.imported} != ${c.expected.imported}`);
  check(r.skipped === c.expected.skipped, `store-rules/${c.id}: skipped ${r.skipped} != ${c.expected.skipped}`);
  check(r.items.length === c.expected.count, `store-rules/${c.id}: count`);
  if (c.expected.stackedCount !== undefined) {
    check(r.items.filter((i) => i.stackOrder !== null).length === c.expected.stackedCount, `store-rules/${c.id}: stackedCount`);
  }
}

// ---- sync-merge
for (const c of fixtures["sync-merge"].cases) {
  const r = simulateSync(c.local, c.remote);
  if (c.mergeTwice) {
    const second = simulateSync(r.items, []);
    check(r.imported === c.expected.firstImported, `sync-merge/${c.id}: firstImported`);
    check(second.imported === c.expected.secondImported, `sync-merge/${c.id}: secondImported`);
  } else {
    check(r.imported === c.expected.imported, `sync-merge/${c.id}: imported ${r.imported} != ${c.expected.imported}`);
  }
  if (c.expected.skipped !== undefined) check(r.skipped === c.expected.skipped, `sync-merge/${c.id}: skipped ${r.skipped} != ${c.expected.skipped}`);
  const contents = r.items.map((i) => i.content);
  check(JSON.stringify(contents) === JSON.stringify(c.expected.contents), `sync-merge/${c.id}: contents got ${JSON.stringify(contents)}, want ${JSON.stringify(c.expected.contents)}`);
  if (c.expected.stackedCount !== undefined) {
    check(r.items.filter((i) => i.stackOrder !== null).length === c.expected.stackedCount, `sync-merge/${c.id}: stackedCount`);
  }
}

// ---- hooks
for (const c of fixtures.hooks.parsing) {
  const got = parseHookLine(c.line);
  const ok = c.expected === null ? got === null : got && got.event === c.expected.event && got.command === c.expected.command;
  check(ok, `hooks/${c.id}: got ${JSON.stringify(got)}, want ${JSON.stringify(c.expected)}`);
}

// ---- accelerators
for (const c of fixtures.accelerators.recording) {
  check(recordAccel(c.event) === c.expected, `accelerators/${c.id}: got ${recordAccel(c.event)}, want ${c.expected}`);
}
for (const c of fixtures.accelerators.display) {
  check(prettyAccel(c.accel) === c.pretty, `accelerators/display ${c.accel}: got ${prettyAccel(c.accel)}, want ${c.pretty}`);
}

// ---- autostart
for (const c of fixtures.autostart.cases) {
  if (c.expectedFile) {
    const exe = c.appImage ?? c.execPath;
    check(renderDesktopEntry(exe) === c.expectedFile.content, `autostart/${c.id}: desktop entry mismatch`);
  } else if ("expectedLoginItemSettings" in c) {
    check(typeof c.expectedLoginItemSettings === "boolean", `autostart/${c.id}: delegation boolean`);
  } else {
    check(c.expectedFile === null, `autostart/${c.id}: expected removal`);
  }
}

// ---- examples vs schema required fields
const storeSchema = load("schema/clipdeck-store.schema.json");
const sampleStore = JSON.parse(readFileSync(join(SPEC, "examples/sample-store.json"), "utf8"));
check(storeSchema.required.every((k) => k in sampleStore), "sample-store: top-level required fields present");
const itemReq = storeSchema.$defs.clipItem.required;
for (const it of sampleStore.items) check(itemReq.every((k) => k in it), `sample-store: item ${it.id} has required fields`);
const settingsReq = storeSchema.$defs.settings.required;
check(settingsReq.every((k) => k in sampleStore.settings), "sample-store: settings required fields present");

const syncSchema = load("schema/sync-line.schema.json");
const syncLines = readFileSync(join(SPEC, "examples/sample-sync.jsonl"), "utf8").trim().split("\n");
for (const [n, line] of syncLines.entries()) {
  const row = JSON.parse(line);
  check(row.v === 1 && syncSchema.required.every((k) => k in row), `sample-sync line ${n}: envelope`);
  check(syncSchema.properties.item.required.every((k) => k in row.item), `sample-sync line ${n}: item fields`);
}

// ---- settings-defaults used by examples
const defaults = fixtures["settings-defaults"].defaults;
for (const [k, v] of Object.entries(defaults)) {
  if (k in sampleStore.settings) {
    check(JSON.stringify(sampleStore.settings[k]) === JSON.stringify(v), `sample-store settings.${k} matches defaults`);
  }
}

console.log(`\n${checks} checks, ${failures} failures`);
process.exit(failures ? 1 : 0);
