import { contextBridge, ipcRenderer } from "electron";
import type { ClipItem, Settings } from "./store";

export interface ClipDeckApi {
  listItems(query?: string, pinboardId?: string | null): Promise<ClipItem[]>;
  copyItem(id: string): Promise<ClipItem | null>;
  pasteItem(id: string, plainText?: boolean): Promise<boolean>;
  deleteItem(id: string): Promise<boolean>;
  pinItem(id: string, pinned: boolean): Promise<ClipItem | null>;
  updateItem(id: string, patch: Partial<ClipItem>): Promise<ClipItem | null>;
  reorder(ids: string[]): Promise<boolean>;
  pushStack(id: string): Promise<ClipItem | null>;
  popStack(): Promise<ClipItem | null>;
  listPinboards(): Promise<{ id: string; name: string; icon: string; position: number }[]>;
  addPinboard(name: string): Promise<{ id: string; name: string; icon: string; position: number }>;
  removePinboard(id: string): Promise<boolean>;
  getSettings(): Promise<Settings>;
  setSettings(patch: Partial<Settings>): Promise<Settings>;
  hide(): Promise<void>;
  openSettings(): Promise<void>;
  clearHistory(): Promise<boolean>;
  appInfo(): Promise<{ platform: string; lastApp: string | null; version: string }>;
  getImage(file: string): Promise<string | null>;
  onItemsChanged(cb: () => void): () => void;
  onPlaySound(cb: (kind: "copy" | "paste") => void): () => void;
}

const api: ClipDeckApi = {
  listItems: (query, pinboardId) => ipcRenderer.invoke("items:list", query, pinboardId ?? null),
  copyItem: (id) => ipcRenderer.invoke("items:copy", id),
  pasteItem: (id, plainText) => ipcRenderer.invoke("items:paste", id, plainText),
  deleteItem: (id) => ipcRenderer.invoke("items:delete", id),
  pinItem: (id, pinned) => ipcRenderer.invoke("items:pin", id, pinned),
  updateItem: (id, patch) => ipcRenderer.invoke("items:update", id, patch),
  reorder: (ids) => ipcRenderer.invoke("items:reorder", ids),
  pushStack: (id) => ipcRenderer.invoke("items:pushStack", id),
  popStack: () => ipcRenderer.invoke("items:popStack"),
  listPinboards: () => ipcRenderer.invoke("pinboards:list"),
  addPinboard: (name) => ipcRenderer.invoke("pinboards:add", name),
  removePinboard: (id) => ipcRenderer.invoke("pinboards:remove", id),
  getSettings: () => ipcRenderer.invoke("settings:get"),
  setSettings: (patch) => ipcRenderer.invoke("settings:set", patch),
  hide: () => ipcRenderer.invoke("app:hide"),
  openSettings: () => ipcRenderer.invoke("app:openSettings"),
  clearHistory: () => ipcRenderer.invoke("app:clearHistory"),
  appInfo: () => ipcRenderer.invoke("app:info"),
  getImage: (file: string) => ipcRenderer.invoke("image:dataUrl", file),
  onItemsChanged: (cb) => {
    const h = () => cb();
    ipcRenderer.on("items-changed", h);
    return () => ipcRenderer.removeListener("items-changed", h);
  },
  onPlaySound: (cb) => {
    const h = (_e: unknown, kind: "copy" | "paste") => cb(kind);
    ipcRenderer.on("play-sound", h);
    return () => ipcRenderer.removeListener("play-sound", h);
  },
};

contextBridge.exposeInMainWorld("clipdeck", api);
