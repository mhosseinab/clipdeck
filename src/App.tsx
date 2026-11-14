import { useCallback, useEffect, useRef, useState } from "react";
import type { ClipItem, Pinboard, Settings } from "./types";
import { playSound } from "./sounds";

const TYPE_LABEL: Record<string, string> = {
  text: "Text",
  link: "Link",
  code: "Code",
  color: "Color",
  image: "Image",
  file: "Files",
  html: "Rich text",
};

function timeAgo(ts: number): string {
  const s = Math.floor((Date.now() - ts) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

const IconPlus = () => (
  <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
    <path d="M6 1v10M1 6h10" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
  </svg>
);

const IconGear = () => (
  <svg width="13" height="13" viewBox="0 0 16 16" fill="none">
    <circle cx="8" cy="8" r="2.4" stroke="currentColor" strokeWidth="1.5" />
    <path
      d="M8 1.5l.9 2.1 2.2-.5 1.1 1.9-1.5 1.7 1.5 1.7-1.1 1.9-2.2-.5-.9 2.1h-.1l-.9-2.1-2.2.5-1.1-1.9L4.2 8 2.7 6.3l1.1-1.9 2.2.5.9-2.1H8"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinejoin="round"
    />
  </svg>
);

const IconStar = ({ filled }: { filled: boolean }) => (
  <svg width="11" height="11" viewBox="0 0 12 12" fill={filled ? "currentColor" : "none"}>
    <path
      d="M6 1l1.5 3.2 3.5.4-2.6 2.4.7 3.4L6 8.7 2.9 10.4l.7-3.4L1 4.6l3.5-.4L6 1z"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinejoin="round"
    />
  </svg>
);

const IconX = () => (
  <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
    <path d="M1.5 1.5l7 7M8.5 1.5l-7 7" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
  </svg>
);

function Card({
  item,
  selected,
  onSelect,
  onPaste,
  onCopy,
  onDelete,
  onPin,
}: {
  item: ClipItem;
  selected: boolean;
  onSelect: () => void;
  onPaste: () => void;
  onCopy: () => void;
  onDelete: () => void;
  onPin: () => void;
}) {
  const [img, setImg] = useState<string | null>(null);
  useEffect(() => {
    if (item.type === "image" && item.imageFile) {
      void window.clipdeck.getImage(item.imageFile).then(setImg);
    }
  }, [item]);

  return (
    <div
      className={"card" + (selected ? " selected" : "")}
      onClick={onSelect}
      onDoubleClick={onPaste}
      onContextMenu={(e) => {
        e.preventDefault();
        onCopy();
      }}
    >
      <div className="card-head">
        <button
          className={"card-btn" + (item.pinned ? " pinned" : "")}
          title={item.pinned ? "Unpin" : "Pin"}
          onClick={(e) => {
            e.stopPropagation();
            onPin();
          }}
        >
          <IconStar filled={item.pinned} />
        </button>
        <button
          className="card-btn"
          title="Delete"
          onClick={(e) => {
            e.stopPropagation();
            onDelete();
          }}
        >
          <IconX />
        </button>
      </div>
      <div className={"card-preview type-" + item.type}>
        {item.type === "image" && img && <img src={img} alt="" />}
        {item.type === "color" && (
          <div className="color-swatch" style={{ background: item.content.trim() }} />
        )}
        {item.type === "link" && (
          <div className="link-preview">
            <div className="favicon-tile">{item.preview.slice(0, 1).toUpperCase()}</div>
            <div className="link-title">{item.title}</div>
            <div className="link-host">{item.preview}</div>
          </div>
        )}
        {item.type === "file" && (
          <div className="file-preview">
            {item.content.split("\n").slice(0, 4).map((f) => (
              <div key={f} className="file-row">
                📄 {f.split("/").pop()}
              </div>
            ))}
            {item.content.split("\n").length > 4 && (
              <div className="file-row">+{item.content.split("\n").length - 4} more</div>
            )}
          </div>
        )}
        {(item.type === "code" || item.type === "text" || item.type === "html") && (
          <pre className={item.type === "code" ? "code-preview" : "text-preview"}>
            {item.content.slice(0, 600)}
          </pre>
        )}
      </div>
      <div className="card-meta">
        <span className={"type-badge t-" + item.type}>{TYPE_LABEL[item.type] ?? item.type}</span>
        <span className="meta-dim">{timeAgo(item.createdAt)}</span>
        {item.stackOrder ? <span className="stack-chip">#{item.stackOrder}</span> : null}
        {item.sourceApp && <span className="meta-dim app">{item.sourceApp}</span>}
      </div>
    </div>
  );
}

export default function App() {
  const [items, setItems] = useState<ClipItem[]>([]);
  const [pinboards, setPinboards] = useState<Pinboard[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const [filter, setFilter] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback(() => {
    void window.clipdeck.listItems(query, filter === "pinned" ? null : filter).then((list) => {
      setItems(filter === "pinned" ? list.filter((i) => i.pinned) : list);
    });
    void window.clipdeck.listPinboards().then(setPinboards);
  }, [query, filter]);

  useEffect(() => {
    refresh();
    void window.clipdeck.getSettings().then(setSettings);
    const off1 = window.clipdeck.onItemsChanged(refresh);
    const off2 = window.clipdeck.onPlaySound((kind) => playSound(kind));
    return () => {
      off1();
      off2();
    };
  }, [refresh]);

  useEffect(() => setSelected(0), [query, filter]);

  const act = async (
    action: "paste" | "copy" | "delete" | "pin" | "stack",
    item: ClipItem,
    plainText?: boolean,
  ) => {
    if (action === "paste") {
      await window.clipdeck.pasteItem(item.id, plainText);
      if (settings?.soundPaste !== false) playSound("paste");
    } else if (action === "copy") {
      await window.clipdeck.copyItem(item.id);
      if (settings?.soundCopy !== false) playSound("copy");
    } else if (action === "delete") {
      await window.clipdeck.deleteItem(item.id);
      refresh();
    } else if (action === "pin") {
      await window.clipdeck.pinItem(item.id, !item.pinned);
      refresh();
    } else if (action === "stack") {
      await window.clipdeck.pushStack(item.id);
      refresh();
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") return void window.clipdeck.hide();
      if (e.key === "ArrowRight" || (e.key === "Tab" && !e.shiftKey)) {
        e.preventDefault();
        setSelected((s) => Math.min(items.length - 1, s + 1));
      } else if (e.key === "ArrowLeft" || (e.key === "Tab" && e.shiftKey)) {
        e.preventDefault();
        setSelected((s) => Math.max(0, s - 1));
      } else if (e.key === "Enter" && items[selected]) {
        e.preventDefault();
        void act("paste", items[selected], e.shiftKey ? true : undefined);
      } else if ((e.metaKey || e.ctrlKey) && e.key === "c" && items[selected]) {
        e.preventDefault();
        void act("copy", items[selected]);
      } else if ((e.metaKey || e.ctrlKey) && e.key === "s" && items[selected]) {
        e.preventDefault();
        void act("stack", items[selected]);
      } else if (e.key === "Delete" && items[selected]) {
        void act("delete", items[selected]);
      } else if ((e.metaKey || e.ctrlKey) && e.key === "f") {
        e.preventDefault();
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [items, selected]);

  const stackCount = items.filter((i) => i.stackOrder !== null).length;

  return (
    <div className="sheet">
      <div className="body">
        <nav className="rail">
          <button className={"rail-item" + (filter === null ? " active" : "")} onClick={() => setFilter(null)}>
            <span className="rail-icon">⏱</span> History
          </button>
          <button className={"rail-item" + (filter === "pinned" ? " active" : "")} onClick={() => setFilter("pinned")}>
            <span className="rail-icon">★</span> Pinned
          </button>
          <div className="rail-section">Pinboards</div>
          {pinboards.map((p) => (
            <button
              key={p.id}
              className={"rail-item" + (filter === p.id ? " active" : "")}
              onClick={() => setFilter(p.id)}
            >
              <span className="rail-icon">▦</span> {p.name}
            </button>
          ))}
          <button
            className="rail-item ghost"
            onClick={() => {
              const name = window.prompt("Pinboard name");
              if (name) void window.clipdeck.addPinboard(name).then(refresh);
            }}
          >
            <span className="rail-icon">
              <IconPlus />
            </span>{" "}
            New
          </button>
        </nav>

        <div className="main">
          <div className="topbar">
            <input
              ref={searchRef}
              className="search"
              placeholder="Search history…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              autoFocus
            />
            <div className="pills">
              {stackCount > 0 && (
                <button
                  className="pill stack"
                  title="Paste next from stack"
                  onClick={() => void window.clipdeck.popStack()}
                >
                  Stack ×{stackCount}
                </button>
              )}
              <button
                className="pill"
                title="New pinboard"
                onClick={() => {
                  const name = window.prompt("Pinboard name");
                  if (name) void window.clipdeck.addPinboard(name).then(refresh);
                }}
              >
                <IconPlus /> Pinboard
              </button>
              <button className="pill icon-pill" title="Settings" onClick={() => void window.clipdeck.openSettings()}>
                <IconGear />
              </button>
            </div>
          </div>

          <div className="strip">
            {items.length === 0 && (
              <div className="empty">
                <div className="empty-glyph">⏱</div>
                <div className="empty-title">Clipboard history is empty</div>
                <div className="empty-sub">Copy anything — it shows up here instantly.</div>
              </div>
            )}
            {items.map((item, idx) => (
              <Card
                key={item.id}
                item={item}
                selected={idx === selected}
                onSelect={() => setSelected(idx)}
                onPaste={() => void act("paste", item)}
                onCopy={() => void act("copy", item)}
                onDelete={() => void act("delete", item)}
                onPin={() => void act("pin", item)}
              />
            ))}
          </div>

          <div className="hints">
            <span>
              <kbd>←</kbd>
              <kbd>→</kbd> navigate
            </span>
            <span>
              <kbd>⏎</kbd> paste
            </span>
            <span>
              <kbd>⇧⏎</kbd> paste plain
            </span>
            <span>
              <kbd>⌘C</kbd> copy
            </span>
            <span>
              <kbd>⌘S</kbd> stack
            </span>
            <span>
              <kbd>⌫</kbd> delete
            </span>
            <span>
              <kbd>esc</kbd> close
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
