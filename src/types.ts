export type ItemType = "text" | "link" | "code" | "color" | "image" | "file" | "html";

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
  stackList(): Promise<ClipItem[]>;
  unstack(id: string): Promise<ClipItem | null>;
  clearStack(): Promise<boolean>;
  listPinboards(): Promise<Pinboard[]>;
  addPinboard(name: string): Promise<Pinboard>;
  removePinboard(id: string): Promise<boolean>;
  getSettings(): Promise<Settings>;
  setSettings(patch: Partial<Settings>): Promise<Settings>;
  hide(): Promise<void>;
  openSettings(): Promise<void>;
  clearHistory(): Promise<boolean>;
  appInfo(): Promise<{ platform: string; lastApp: string | null; version: string }>;
  getImage(file: string): Promise<string | null>;
  getFavicon(url: string): Promise<string | null>;
  startDrag(file: string): void;
  exportHistory(): Promise<string | null>;
  importHistory(): Promise<{ imported: number; skipped: number } | null>;
  checkUpdates(): Promise<string>;
  onItemsChanged(cb: () => void): () => void;
  onPlaySound(cb: (kind: "copy" | "paste") => void): () => void;
}

declare global {
  interface Window {
    clipdeck: ClipDeckApi;
  }
}
