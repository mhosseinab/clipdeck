import { execFile } from "node:child_process";
import { promisify } from "node:util";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { clipboard, nativeImage } from "electron";
import type { ClipItem, ItemType, Store } from "./store";

const exec = promisify(execFile);
const isMac = process.platform === "darwin";

/* ---------------- platform helpers ---------------- */

export async function frontmostApp(): Promise<string | null> {
  try {
    if (isMac) {
      const { stdout } = await exec("osascript", [
        "-e",
        'tell application "System Events" to get name of first application process whose frontmost is true',
      ]);
      return stdout.trim() || null;
    }
    const { stdout } = await exec("xdotool", ["getactivewindow", "getwindowname"]);
    return stdout.trim() || null;
  } catch {
    return null;
  }
}

export async function simulatePaste(): Promise<boolean> {
  try {
    if (isMac) {
      await exec("osascript", [
        "-e",
        'tell application "System Events" to keystroke "v" using command down',
      ]);
      return true;
    }
    const wl = process.env.WAYLAND_DISPLAY;
    if (wl) {
      try {
        await exec("wtype", ["-M", "ctrl", "-k", "v", "-m", "ctrl"]);
        return true;
      } catch {
        await exec("ydotool", ["key", "29:1", "47:1", "47:0", "29:0"]);
        return true;
      }
    }
    await exec("xdotool", ["key", "--clearmodifiers", "ctrl+v"]);
    return true;
  } catch {
    return false;
  }
}

async function readClipFiles(): Promise<string[]> {
  try {
    if (isMac) {
      const script = `
        ObjC.import("AppKit");
        var p = $.NSPasteboard.generalPasteboard;
        var urls = p.readObjectsForClassesOptions([$.NSURL], $());
        var out = [];
        if (urls) for (var i = 0; i < urls.count; i++) out.push(ObjC.unwrap(urls.objectAtIndex(i).path));
        out.join("\\n");`;
      const { stdout } = await exec("osascript", ["-l", "JavaScript", "-e", script]);
      return stdout.split("\n").map((s) => s.trim()).filter(Boolean);
    }
    const cmd = process.env.WAYLAND_DISPLAY ? "wl-paste" : "xclip";
    const args = process.env.WAYLAND_DISPLAY
      ? ["-t", "text/uri-list", "-n"]
      : ["-selection", "clipboard", "-t", "text/uri-list", "-o"];
    const { stdout } = await exec(cmd, args);
    return stdout
      .split("\n")
      .map((s) => decodeURIComponent(s.trim()))
      .filter((s) => s.startsWith("file://"))
      .map((s) => s.slice("file://".length));
  } catch {
    return [];
  }
}

/* ---------------- capture ---------------- */

function classifyText(text: string): ItemType {
  const t = text.trim();
  if (/^#[0-9a-fA-F]{3,8}$/.test(t) || /^rgba?\(/.test(t)) return "color";
  if (/^(https?:\/\/|www\.)\S+$/.test(t)) return "link";
  const codeScore =
    (/[;{}()[\]]/.test(t) ? 1 : 0) +
    (/(const |let |var |function |def |class |import |#include|<\/|\/>)/.test(t) ? 1 : 0) +
    (/\n/.test(t) && / {2,}|\t/.test(text) ? 1 : 0);
  return codeScore >= 2 ? "code" : "text";
}

function previewOf(type: ItemType, content: string): string {
  if (type === "link") {
    try {
      return new URL(content).hostname.replace(/^www\./, "");
    } catch {
      return content.slice(0, 80);
    }
  }
  const firstLine = content.split("\n")[0].trim();
  return firstLine.length > 120 ? firstLine.slice(0, 120) + "…" : firstLine;
}

function titleOf(type: ItemType, content: string): string {
  if (type === "link") return previewOf("link", content);
  if (type === "color") return content.trim();
  return content.split("\n")[0].trim().slice(0, 60) || type;
}

function hash(buf: Buffer | string): string {
  return crypto.createHash("sha256").update(buf).digest("hex").slice(0, 32);
}

const TRANSIENT_TYPES = [
  "org.nspasteboard.TransientType",
  "de.petermaurer.TransientPasteboardType",
  "com.agilebits.onepassword",
];

export class ClipboardWatcher {
  private timer: NodeJS.Timeout | null = null;
  private lastSig = "";
  private lastImageHash = "";
  private lastImageCheck = 0;
  private stopped = true;

  constructor(
    private store: Store,
    private onItem: (item: ClipItem) => void,
  ) {}

  start(intervalMs = 400): void {
    if (!this.stopped) return;
    this.stopped = false;
    const tick = async () => {
      if (this.stopped) return;
      await this.poll().catch(() => {});
      this.timer = setTimeout(tick, intervalMs);
    };
    void tick();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private async poll(): Promise<void> {
    const settings = this.store.settings();
    if (!settings.watchEnabled) return;
    const formats = clipboard.availableFormats().sort().join(",");
    if (settings.ignoreTransient && TRANSIENT_TYPES.some((t) => formats.includes(t))) {
      this.lastSig = formats + "|transient";
      return;
    }
    const text = clipboard.readText() ?? "";
    const sig = formats + "|" + hash(text);
    const hasImage = /image\//.test(formats);
    const now = Date.now();

    let imageChanged = false;
    if (hasImage && (sig !== this.lastSig || now - this.lastImageCheck > 2000)) {
      this.lastImageCheck = now;
      const img = clipboard.readImage();
      if (!img.isEmpty()) {
        const h = hash(img.toPNG());
        imageChanged = h !== this.lastImageHash && sig !== this.lastSig;
        this.lastImageHash = h;
      }
    }

    if (sig === this.lastSig && !imageChanged) return;
    const prevSig = this.lastSig;
    this.lastSig = sig;
    if (!prevSig) return; // first observation: don't record whatever was there at launch

    const files = await readClipFiles();
    if (files.length > 0) {
      this.record({
        type: "file",
        content: files.join("\n"),
        preview: files.length === 1 ? path.basename(files[0]) : `${files.length} files`,
        title: files.length === 1 ? path.basename(files[0]) : `${files.length} files`,
        sizeBytes: 0,
      });
      return;
    }

    const img = hasImage ? clipboard.readImage() : nativeImage.createEmpty();
    if (!img.isEmpty()) {
      const png = img.toPNG();
      const file = hash(png) + ".png";
      fs.writeFileSync(path.join(this.store.images, file), png);
      this.record({
        type: "image",
        content: file,
        preview: `${img.getSize().width}×${img.getSize().height} image`,
        title: "Image",
        sizeBytes: png.length,
        imageFile: file,
      });
      return;
    }

    const html = clipboard.readHTML() ?? "";
    const body = html ? text || html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim() : text;
    if (!body.trim()) return;
    const type = classifyText(body);
    this.record({
      type: html && type === "text" ? "html" : type,
      content: body,
      preview: previewOf(type, body),
      title: titleOf(type, body),
      sizeBytes: Buffer.byteLength(body),
    });
  }

  private record(partial: Omit<ClipItem, "id" | "createdAt" | "sourceApp" | "pinned" | "pinboardId" | "stackOrder">): void {
    const item: ClipItem = {
      id: crypto.randomUUID(),
      createdAt: Date.now(),
      sourceApp: null,
      pinned: false,
      pinboardId: null,
      stackOrder: null,
      ...partial,
    } as ClipItem;
    void frontmostApp().then((app) => {
      const settings = this.store.settings();
      const blocked = settings.excludedApps.some(
        (a) => app && a && app.toLowerCase().includes(a.toLowerCase()),
      );
      const confidentialApp = settings.ignoreConfidential && app
        ? /keychain|password|1password|bitwarden|keepass|lastpass/i.test(app)
        : false;
      if (blocked || confidentialApp) return;
      item.sourceApp = app;
      const saved = this.store.addItem(item);
      if (saved === item) this.onItem(item);
    });
  }
}

export function writeToClipboard(item: ClipItem, store: Store, plainOnly = false): void {
  if (!plainOnly && item.type === "image" && item.imageFile) {
    const p = path.join(store.images, item.imageFile);
    if (fs.existsSync(p)) {
      clipboard.writeImage(nativeImage.createFromPath(p));
      return;
    }
  }
  if (!plainOnly && item.type === "html" && /<[a-z][\s\S]*>/i.test(item.content)) {
    clipboard.writeHTML(item.content);
    return;
  }
  clipboard.writeText(item.content);
}
