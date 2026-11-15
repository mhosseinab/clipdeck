import {
  app,
  BrowserWindow,
  dialog,
  globalShortcut,
  ipcMain,
  Menu,
  nativeImage,
  screen,
  Tray,
} from "electron";
import path from "node:path";
import fs from "node:fs";
import zlib from "node:zlib";
import { Store, type ClipItem, type Settings } from "./store";
import { ClipboardWatcher, frontmostApp, simulatePaste, writeToClipboard } from "./clipboard";
import { runHooks } from "./hooks";
import { SyncStore } from "./sync";

// Test/dev hook: run a parallel instance without touching the real profile.
if (process.env.CLIPDECK_USER_DATA) {
  app.setPath("userData", process.env.CLIPDECK_USER_DATA);
}

let store: Store;
let sync: SyncStore;
let watcher: ClipboardWatcher;
let overlay: BrowserWindow | null = null;
let settingsWin: BrowserWindow | null = null;
let tray: Tray | null = null;
let lastApp: string | null = null;
let watching = true;

/* ---------- original runtime-generated icon (no external assets) ---------- */
function makeIcon(): Electron.NativeImage {
  const size = 32;
  const px = Buffer.alloc(size * size * 4);
  const put = (x: number, y: number, r: number, g: number, b: number, a = 255) => {
    if (x < 0 || y < 0 || x >= size || y >= size) return;
    const o = (y * size + x) * 4;
    px[o] = r; px[o + 1] = g; px[o + 2] = b; px[o + 3] = a;
  };
  for (let y = 4; y < 28; y++)
    for (let x = 4; x < 28; x++) {
      const corner = (x < 7 || x > 24) && (y < 7 || y > 24);
      if (corner && Math.hypot(x < 16 ? 7 - x : x - 24, y < 16 ? 7 - y : y - 24) > 3.6) continue;
      put(x, y, 24, 26, 38);
    }
  for (let y = 8; y < 12; y++) for (let x = 11; x < 21; x++) put(x, y, 122, 162, 255);
  for (let y = 14; y < 24; y++) for (let x = 8; x < 24; x++) put(x, y, 240, 242, 250);
  for (let y = 16; y < 22; y += 3) for (let x = 11; x < 21; x++) put(x, y, 140, 146, 166);
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    px.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  }
  const chunk = (type: string, data: Buffer) => {
    const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const crcTable = Array.from({ length: 256 }, (_, n) => {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      return c >>> 0;
    });
    let crc = 0xffffffff;
    for (const b of body) crc = crcTable[(crc ^ b) & 0xff] ^ (crc >>> 8);
    const crcBuf = Buffer.alloc(4); crcBuf.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
    return Buffer.concat([len, body, crcBuf]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; ihdr[9] = 6;
  const png = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
  return nativeImage.createFromBuffer(png);
}

/* ---------- windows ---------- */

function applyScreenShare(): void {
  if (!overlay) return;
  overlay.setContentProtection(!store.settings().showInScreenShare);
}

function createOverlay(): void {
  const { width, height } = screen.getPrimaryDisplay().workAreaSize;
  overlay = new BrowserWindow({
    width: Math.min(1240, width - 80),
    height: 372,
    x: Math.round((width - Math.min(1240, width - 80)) / 2),
    y: height - 372 - 12,
    frame: false,
    transparent: true,
    resizable: false,
    maximizable: false,
    fullscreenable: false,
    skipTaskbar: true,
    alwaysOnTop: true,
    hasShadow: false,
    show: false,
    backgroundColor: "#00000000",
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
    },
  });
  overlay.on("blur", () => overlay?.hide());
  applyScreenShare();
  const dev = process.env.VITE_DEV_SERVER_URL;
  void overlay.loadURL(dev ?? `file://${path.join(__dirname, "../dist/index.html")}`);
}

function showOverlay(): void {
  if (!overlay) return;
  const { width, height } = screen.getPrimaryDisplay().workAreaSize;
  const w = Math.min(1240, width - 80);
  overlay.setBounds({
    x: Math.round((width - w) / 2),
    y: height - 372 - 12,
    width: w,
    height: 372,
  });
  void frontmostApp().then((name) => {
    lastApp = name;
    overlay?.show();
    overlay?.focus();
    overlay?.webContents.send("items-changed");
  });
}

function hideOverlay(): void {
  overlay?.hide();
}

function createSettingsWindow(): void {
  if (settingsWin) {
    settingsWin.focus();
    return;
  }
  settingsWin = new BrowserWindow({
    width: 600,
    height: 560,
    frame: true,
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
    },
  });
  void settingsWin.loadURL(
    (process.env.VITE_DEV_SERVER_URL ?? `file://${path.join(__dirname, "../dist/index.html")}`) +
      "#/settings",
  );
  settingsWin.on("closed", () => (settingsWin = null));
}

/* ---------- tray ---------- */

function createTray(): void {
  if (!tray) tray = new Tray(makeIcon());
  tray.setToolTip("ClipDeck");
  const recents = store.recentItems(8).map((item) => ({
    label: (item.title || item.preview || item.type).slice(0, 42),
    click: () => {
      writeToClipboard(item, store);
      if (store.settings().soundCopy) overlay?.webContents.send("play-sound", "copy");
    },
  }));
  tray.setContextMenu(
    Menu.buildFromTemplate([
      { label: "Open ClipDeck", click: () => showOverlay() },
      { label: watching ? "Pause watching" : "Resume watching", click: toggleWatching },
      ...(recents.length
        ? [{ type: "separator" as const }, { label: "Recent", enabled: false }, ...recents]
        : []),
      { type: "separator" },
      { label: "Settings…", click: () => createSettingsWindow() },
      {
        label: "Clear history",
        click: () => {
          store.clearHistory();
          overlay?.webContents.send("items-changed");
        },
      },
      { type: "separator" },
      { label: "Quit", click: () => app.quit() },
    ]),
  );
  tray.on("click", () => showOverlay());
}

let trayRefreshTimer: NodeJS.Timeout | null = null;
function scheduleTrayRefresh(): void {
  if (trayRefreshTimer) return;
  trayRefreshTimer = setTimeout(() => {
    trayRefreshTimer = null;
    if (tray) createTray();
  }, 1000);
}

function toggleWatching(): void {
  watching = !watching;
  store.setSettings({ watchEnabled: watching });
  if (watching) watcher.start();
  else watcher.stop();
  createTray();
}

/* ---------- updates (scaffold) ---------- */

function checkUpdates(): Promise<string> {
  if (!app.isPackaged) return Promise.resolve("Update checks run in packaged builds only.");
  try {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const { autoUpdater } = require("electron-updater") as typeof import("electron-updater");
    autoUpdater.autoDownload = false;
    return new Promise((resolve) => {
      const done = (msg: string) => {
        autoUpdater.removeAllListeners();
        resolve(msg);
      };
      autoUpdater.once("update-available", (info: { version?: string }) =>
        done(`Update available: v${info?.version ?? "?"}`),
      );
      autoUpdater.once("update-not-available", () => done("You are on the latest version."));
      autoUpdater.once("error", (e: Error) => done("Update check failed: " + e.message));
      setTimeout(() => done("Update check timed out."), 15000);
      void autoUpdater.checkForUpdates();
    });
  } catch {
    return Promise.resolve("Updater unavailable.");
  }
}

/* ---------- IPC ---------- */

function wireIpc(): void {
  ipcMain.handle("items:list", (_e, query?: string, pinboardId?: string | null) => {
    let items = store.items();
    if (pinboardId) items = items.filter((i) => i.pinboardId === pinboardId);
    if (query && query.trim()) {
      const q = query.trim().toLowerCase();
      items = items.filter(
        (i) =>
          i.title.toLowerCase().includes(q) ||
          i.preview.toLowerCase().includes(q) ||
          i.content.toLowerCase().includes(q),
      );
    }
    return items.slice(0, 300);
  });
  ipcMain.handle("items:copy", (_e, id: string) => {
    const item = store.find(id);
    if (!item) return null;
    writeToClipboard(item, store);
    runHooks("copy", item, store.settings());
    return item;
  });
  ipcMain.handle("items:paste", async (_e, id: string, plainText?: boolean) => {
    const item = store.find(id);
    if (!item) return false;
    const s = store.settings();
    const rulePlain =
      lastApp != null &&
      s.plainTextApps.some((a) => a && lastApp!.toLowerCase().includes(a.toLowerCase()));
    writeToClipboard(item, store, plainText ?? (s.pasteAsPlainText || rulePlain));
    runHooks("paste", item, s);
    hideOverlay();
    await new Promise((r) => setTimeout(r, 60));
    return simulatePaste();
  });
  ipcMain.handle("items:delete", (_e, id: string) => {
    store.remove(id);
    scheduleTrayRefresh();
    return true;
  });
  ipcMain.handle("items:pin", (_e, id: string, pinned: boolean) => {
    return store.update(id, { pinned });
  });
  ipcMain.handle("items:update", (_e, id: string, patch: Partial<ClipItem>) => {
    return store.update(id, patch);
  });
  ipcMain.handle("items:reorder", (_e, ids: string[]) => {
    store.reorder(ids);
    scheduleTrayRefresh();
    return true;
  });
  ipcMain.handle("items:pushStack", (_e, id: string) => {
    const stackLen = store.items().filter((i) => i.stackOrder !== null).length;
    return store.update(id, { stackOrder: stackLen + 1 });
  });
  ipcMain.handle("items:popStack", async () => {
    const queued = store
      .items()
      .filter((i) => i.stackOrder !== null)
      .sort((a, b) => (a.stackOrder ?? 0) - (b.stackOrder ?? 0));
    const next = store.settings().stackDirection === "lifo" ? queued[queued.length - 1] : queued[0];
    if (!next) return null;
    writeToClipboard(next, store);
    runHooks("paste", next, store.settings());
    hideOverlay();
    await new Promise((r) => setTimeout(r, 60));
    await simulatePaste();
    store.remove(next.id);
    return next;
  });
  ipcMain.handle("items:stackList", () =>
    store
      .items()
      .filter((i) => i.stackOrder !== null)
      .sort((a, b) => (a.stackOrder ?? 0) - (b.stackOrder ?? 0)),
  );
  ipcMain.handle("items:unstack", (_e, id: string) => store.update(id, { stackOrder: null }));
  ipcMain.handle("items:clearStack", () => {
    for (const i of store.items().filter((x) => x.stackOrder !== null)) {
      store.update(i.id, { stackOrder: null });
    }
    return true;
  });
  ipcMain.handle("pinboards:list", () => store.pinboards());
  ipcMain.handle("pinboards:add", (_e, name: string) => store.addPinboard(name));
  ipcMain.handle("pinboards:remove", (_e, id: string) => {
    store.removePinboard(id);
    return true;
  });
  ipcMain.handle("settings:get", () => store.settings());
  ipcMain.handle("settings:set", (_e, patch: Partial<Settings>) => {
    const before = store.settings();
    const s = store.setSettings(patch);
    if (patch.watchEnabled !== undefined) {
      watching = s.watchEnabled;
      if (watching) watcher.start();
      else watcher.stop();
    }
    if (patch.syncDirectory !== undefined && patch.syncDirectory !== before.syncDirectory) {
      sync.reconfigure();
    }
    if (patch.showInScreenShare !== undefined) applyScreenShare();
    return s;
  });
  ipcMain.handle("app:hide", () => hideOverlay());
  ipcMain.handle("app:openSettings", () => createSettingsWindow());
  ipcMain.handle("app:clearHistory", () => {
    store.clearHistory();
    scheduleTrayRefresh();
    return true;
  });
  ipcMain.handle("app:info", () => ({
    platform: process.platform,
    lastApp,
    version: app.getVersion(),
  }));
  ipcMain.handle("app:export", async () => {
    const win = BrowserWindow.getFocusedWindow() ?? undefined;
    const { canceled, filePath } = await dialog.showSaveDialog(win ?? settingsWin ?? overlay!, {
      defaultPath: "clipdeck-export.json",
      filters: [{ name: "ClipDeck export", extensions: ["json"] }],
    });
    if (canceled || !filePath) return null;
    fs.writeFileSync(filePath, store.exportJSON());
    return filePath;
  });
  ipcMain.handle("app:import", async () => {
    const win = BrowserWindow.getFocusedWindow() ?? undefined;
    const { canceled, filePaths } = await dialog.showOpenDialog(win ?? settingsWin ?? overlay!, {
      filters: [{ name: "ClipDeck export", extensions: ["json"] }],
      properties: ["openFile"],
    });
    if (canceled || !filePaths[0]) return null;
    try {
      const result = store.importJSON(fs.readFileSync(filePaths[0], "utf8"));
      scheduleTrayRefresh();
      return result;
    } catch {
      return null;
    }
  });
  ipcMain.handle("app:checkUpdates", () => checkUpdates());
  ipcMain.handle("image:dataUrl", (_e, file: string) => {
    const safe = path.basename(file);
    const p = path.join(store.images, safe);
    try {
      const buf = fs.readFileSync(p);
      return `data:image/png;base64,${buf.toString("base64")}`;
    } catch {
      return null;
    }
  });
  ipcMain.handle("link:favicon", async (_e, url: string) => {
    if (!store.settings().generateLinkPreviews) return null;
    try {
      const u = new URL(url);
      const key = u.origin;
      const cached = faviconCache.get(key);
      if (cached !== undefined) return cached;
      const ctrl = new AbortController();
      const timer = setTimeout(() => ctrl.abort(), 4000);
      const res = await fetch(u.origin + "/favicon.ico", {
        signal: ctrl.signal,
        redirect: "follow",
      }).catch(() => null);
      clearTimeout(timer);
      let dataUrl: string | null = null;
      if (res?.ok) {
        const type = res.headers.get("content-type") || "image/x-icon";
        const buf = Buffer.from(await res.arrayBuffer());
        if (buf.length > 0 && buf.length < 512 * 1024) {
          dataUrl = `data:${type};base64,${buf.toString("base64")}`;
        }
      }
      faviconCache.set(key, dataUrl);
      return dataUrl;
    } catch {
      return null;
    }
  });
  ipcMain.on("drag:start", (e, file: string) => {
    const inImages = path.dirname(path.resolve(file)) === path.resolve(store.images);
    const inStore = store.items().some((i) => i.content.split("\n").includes(file));
    if ((inImages || inStore) && fs.existsSync(file)) {
      e.sender.startDrag({ file: path.resolve(file), icon: makeIcon() });
    }
  });
}

const faviconCache = new Map<string, string | null>();

/* ---------- lifecycle ---------- */

const gotLock = app.requestSingleInstanceLock();
if (!gotLock) {
  app.quit();
} else {
  app.whenReady().then(() => {
    store = new Store(app.getPath("userData"));
    sync = new SyncStore(store);
    watching = store.settings().watchEnabled;
    watcher = new ClipboardWatcher(store, (item) => {
      sync.append(item);
      scheduleTrayRefresh();
      overlay?.webContents.send("items-changed");
      if (store.settings().soundCopy) overlay?.webContents.send("play-sound", "copy");
    });
    if (watching) watcher.start();

    createOverlay();
    createTray();
    wireIpc();

    setInterval(() => {
      if (sync.merge() > 0) overlay?.webContents.send("items-changed");
    }, 30000);

    try {
      globalShortcut.register(store.settings().shortcut, () => {
        if (overlay?.isVisible()) hideOverlay();
        else showOverlay();
      });
    } catch {
      /* Wayland or restricted session: tray click still works */
    }

    app.on("activate", () => showOverlay());
  });

  app.on("window-all-closed", () => {
    /* keep running in tray */
  });
  app.on("will-quit", () => {
    globalShortcut.unregisterAll();
    watcher?.stop();
  });
  app.on("second-instance", () => showOverlay());
}
