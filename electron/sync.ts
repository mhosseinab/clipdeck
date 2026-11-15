import fs from "node:fs";
import path from "node:path";
import type { ClipItem, Store } from "./store";

/**
 * Folder-based self-hosted sync (T2). Appends new items to a JSONL file in a
 * user-chosen directory (e.g. a Syncthing/Dropbox/NFS folder) and merges
 * items written by other machines, deduplicating by type+content.
 */
export class SyncStore {
  private store: Store;
  private file: string | null = null;
  private lastSize = 0;

  constructor(store: Store) {
    this.store = store;
    this.reconfigure();
  }

  reconfigure(): void {
    const dir = this.store.settings().syncDirectory.trim();
    this.file = dir ? path.join(dir, "clipdeck-sync.jsonl") : null;
    this.lastSize = 0;
    if (this.file) {
      try {
        fs.mkdirSync(dir, { recursive: true });
        this.lastSize = fs.statSync(this.file).size;
      } catch {
        this.lastSize = 0;
      }
    }
  }

  enabled(): boolean {
    return this.file !== null;
  }

  append(item: ClipItem): void {
    if (!this.file) return;
    try {
      fs.appendFileSync(this.file, JSON.stringify({ v: 1, item }) + "\n");
    } catch {
      /* sync is best-effort */
    }
  }

  /** Merge items other machines appended since the last merge. */
  merge(): number {
    if (!this.file) return 0;
    let size = 0;
    try {
      size = fs.statSync(this.file).size;
    } catch {
      return 0;
    }
    if (size <= this.lastSize) return 0;
    let imported = 0;
    try {
      const data = fs.readFileSync(this.file, "utf8");
      const seen = new Set(this.store.items().map((i) => i.type + "|" + i.content));
      for (const line of data.split("\n")) {
        if (!line.trim()) continue;
        try {
          const row = JSON.parse(line) as { item?: ClipItem };
          const it = row.item;
          if (!it?.id || typeof it.content !== "string") continue;
          const key = it.type + "|" + it.content;
          if (seen.has(key)) continue;
          seen.add(key);
          this.store.addItem({ ...it, pinboardId: it.pinboardId ?? null, stackOrder: null });
          imported++;
        } catch {
          /* skip malformed rows */
        }
      }
    } catch {
      /* best-effort */
    }
    this.lastSize = size;
    return imported;
  }
}
