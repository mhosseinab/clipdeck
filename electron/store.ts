import fs from "node:fs";
import path from "node:path";

export type ItemType =
  | "text"
  | "link"
  | "code"
  | "color"
  | "image"
  | "file"
  | "html";

export interface ClipItem {
  id: string;
  type: ItemType;
  content: string;
  preview: string;
  title: string;
  sourceApp: string | null;
  createdAt: number;
  sizeBytes: number;
  pinned: boolean;
  pinboardId: string | null;
  stackOrder: number | null;
  imageFile?: string;
}

export interface Pinboard {
  id: string;
  name: string;
  icon: string;
  position: number;
}

export interface Settings {
  watchEnabled: boolean;
  soundCopy: boolean;
  soundPaste: boolean;
  maxItems: number;
  shortcut: string;
  launchAtLogin: boolean;
  theme: "dark" | "light";
  excludedApps: string[];
  ignoreTransient: boolean;
  ignoreConfidential: boolean;
  generateLinkPreviews: boolean;
  pasteAsPlainText: boolean;
  stackDirection: "fifo" | "lifo";
  plainTextApps: string[];
  syncDirectory: string;
  showInScreenShare: boolean;
  hooks: { event: "copy" | "paste"; command: string }[];
}

export const DEFAULT_SETTINGS: Settings = {
  watchEnabled: true,
  soundCopy: true,
  soundPaste: true,
  maxItems: 5000,
  shortcut: "CommandOrControl+Shift+V",
  launchAtLogin: true,
  theme: "dark",
  excludedApps: ["com.apple.keychainaccess", "com.apple.Passwords", "1Password", "Bitwarden", "KeePass"],
  ignoreTransient: true,
  ignoreConfidential: true,
  generateLinkPreviews: true,
  pasteAsPlainText: false,
  stackDirection: "fifo",
  plainTextApps: [],
  syncDirectory: "",
  showInScreenShare: true,
  hooks: [],
};

interface StoreShape {
  items: ClipItem[];
  pinboards: Pinboard[];
  settings: Settings;
}

export class Store {
  private file: string;
  private imageDir: string;
  private data: StoreShape;
  private saveTimer: NodeJS.Timeout | null = null;

  constructor(userDataDir: string) {
    this.file = path.join(userDataDir, "clipdeck-store.json");
    this.imageDir = path.join(userDataDir, "images");
    fs.mkdirSync(this.imageDir, { recursive: true });
    this.data = { items: [], pinboards: [], settings: { ...DEFAULT_SETTINGS } };
    try {
      const raw = fs.readFileSync(this.file, "utf8");
      const parsed = JSON.parse(raw) as Partial<StoreShape>;
      this.data.items = parsed.items ?? [];
      this.data.pinboards = parsed.pinboards ?? [];
      this.data.settings = { ...DEFAULT_SETTINGS, ...(parsed.settings ?? {}) };
    } catch {
      /* first run */
    }
  }

  get images(): string {
    return this.imageDir;
  }

  private persist(): void {
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => {
      const tmp = this.file + ".tmp";
      fs.writeFileSync(tmp, JSON.stringify(this.data));
      fs.renameSync(tmp, this.file);
    }, 150);
  }

  items(): ClipItem[] {
    return this.data.items;
  }

  find(id: string): ClipItem | undefined {
    return this.data.items.find((i) => i.id === id);
  }

  addItem(item: ClipItem): ClipItem {
    const dupIndex = this.data.items.findIndex(
      (i) => i.type === item.type && i.content === item.content && !i.stackOrder,
    );
    if (dupIndex >= 0) {
      const [existing] = this.data.items.splice(dupIndex, 1);
      existing.createdAt = item.createdAt;
      existing.sourceApp = item.sourceApp;
      this.data.items.unshift(existing);
      this.trim();
      this.persist();
      return existing;
    }
    this.data.items.unshift(item);
    this.trim();
    this.persist();
    return item;
  }

  private trim(): void {
    const max = this.data.settings.maxItems;
    const keep: ClipItem[] = [];
    const overflow: ClipItem[] = [];
    for (const i of this.data.items) {
      (i.pinned || i.pinboardId || i.stackOrder ? keep : overflow).push(i);
    }
    const kept = keep.concat(overflow.slice(0, Math.max(0, max - keep.length)));
    for (const dropped of overflow.slice(Math.max(0, max - keep.length))) {
      if (dropped.imageFile) {
        fs.rm(path.join(this.imageDir, dropped.imageFile), { force: true }, () => {});
      }
    }
    this.data.items = kept;
  }

  update(id: string, patch: Partial<ClipItem>): ClipItem | null {
    const item = this.find(id);
    if (!item) return null;
    Object.assign(item, patch);
    this.persist();
    return item;
  }

  remove(id: string): void {
    const idx = this.data.items.findIndex((i) => i.id === id);
    if (idx < 0) return;
    const [gone] = this.data.items.splice(idx, 1);
    if (gone.imageFile) {
      fs.rm(path.join(this.imageDir, gone.imageFile), { force: true }, () => {});
    }
    this.persist();
  }

  clearHistory(): void {
    for (const i of this.data.items) {
      if (i.imageFile && !i.pinned && !i.pinboardId) {
        fs.rm(path.join(this.imageDir, i.imageFile), { force: true }, () => {});
      }
    }
    this.data.items = this.data.items.filter((i) => i.pinned || i.pinboardId);
    this.persist();
  }

  pinboards(): Pinboard[] {
    return this.data.pinboards;
  }

  addPinboard(name: string, icon = "star"): Pinboard {
    const pb: Pinboard = {
      id: crypto.randomUUID(),
      name,
      icon,
      position: this.data.pinboards.length,
    };
    this.data.pinboards.push(pb);
    this.persist();
    return pb;
  }

  removePinboard(id: string): void {
    this.data.pinboards = this.data.pinboards.filter((p) => p.id !== id);
    for (const i of this.data.items) {
      if (i.pinboardId === id) i.pinboardId = null;
    }
    this.persist();
  }

  settings(): Settings {
    return this.data.settings;
  }

  setSettings(patch: Partial<Settings>): Settings {
    this.data.settings = { ...this.data.settings, ...patch };
    this.persist();
    return this.data.settings;
  }

  reorder(ids: string[]): void {
    const byId = new Map(this.data.items.map((i) => [i.id, i]));
    const ordered: ClipItem[] = [];
    for (const id of ids) {
      const it = byId.get(id);
      if (it) {
        ordered.push(it);
        byId.delete(id);
      }
    }
    this.data.items = ordered.concat([...byId.values()]);
    this.persist();
  }

  setImageFile(id: string, file: string): void {
    const item = this.find(id);
    if (item) {
      item.imageFile = file;
      this.persist();
    }
  }

  exportJSON(): string {
    return JSON.stringify(
      { version: 1, exportedAt: new Date().toISOString(), items: this.data.items, pinboards: this.data.pinboards },
      null,
      2,
    );
  }

  importJSON(raw: string): { imported: number; skipped: number } {
    const parsed = JSON.parse(raw) as Partial<StoreShape> & { items?: ClipItem[] };
    const incoming = parsed.items ?? [];
    const pinboards = parsed.pinboards ?? [];
    let imported = 0;
    let skipped = 0;
    const seen = new Set(this.data.items.map((i) => i.type + "|" + i.content));
    for (const item of incoming) {
      if (!item || typeof item.content !== "string" || !item.id) {
        skipped++;
        continue;
      }
      const key = item.type + "|" + item.content;
      if (seen.has(key)) {
        skipped++;
        continue;
      }
      seen.add(key);
      this.data.items.push({ ...item, pinboardId: item.pinboardId ?? null, stackOrder: null });
      imported++;
    }
    for (const pb of pinboards) {
      if (pb?.id && !this.data.pinboards.some((p) => p.id === pb.id)) this.data.pinboards.push(pb);
    }
    this.data.items.sort((a, b) => b.createdAt - a.createdAt);
    this.persist();
    return { imported, skipped };
  }

  recentItems(n: number): ClipItem[] {
    return this.data.items.slice(0, n);
  }
}
