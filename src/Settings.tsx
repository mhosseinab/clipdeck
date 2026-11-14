import { useEffect, useState } from "react";
import type { Settings } from "./types";

export default function SettingsPage() {
  const [s, setS] = useState<Settings | null>(null);

  useEffect(() => {
    void window.clipdeck.getSettings().then(setS);
  }, []);

  if (!s) return <div className="settings-page">Loading…</div>;

  const patch = (p: Partial<Settings>) => {
    void window.clipdeck.setSettings(p).then(setS);
  };

  return (
    <div className="settings-page">
      <h1>ClipDeck Settings</h1>

      <section>
        <h2>General</h2>
        <label className="row">
          <span>Watch clipboard</span>
          <input type="checkbox" checked={s.watchEnabled} onChange={(e) => patch({ watchEnabled: e.target.checked })} />
        </label>
        <label className="row">
          <span>Maximum history items</span>
          <input
            type="number"
            min={50}
            max={100000}
            value={s.maxItems}
            onChange={(e) => patch({ maxItems: Number(e.target.value) || 5000 })}
          />
        </label>
        <label className="row">
          <span>Global shortcut</span>
          <input value={s.shortcut} onChange={(e) => patch({ shortcut: e.target.value })} />
        </label>
      </section>

      <section>
        <h2>Sounds</h2>
        <label className="row">
          <span>Play sound on copy</span>
          <input type="checkbox" checked={s.soundCopy} onChange={(e) => patch({ soundCopy: e.target.checked })} />
        </label>
        <label className="row">
          <span>Play sound on paste</span>
          <input type="checkbox" checked={s.soundPaste} onChange={(e) => patch({ soundPaste: e.target.checked })} />
        </label>
      </section>

      <section>
        <h2>Content rules</h2>
        <label className="row">
          <span>Ignore transient content (password manager marks)</span>
          <input type="checkbox" checked={s.ignoreTransient} onChange={(e) => patch({ ignoreTransient: e.target.checked })} />
        </label>
        <label className="row">
          <span>Ignore confidential content</span>
          <input type="checkbox" checked={s.ignoreConfidential} onChange={(e) => patch({ ignoreConfidential: e.target.checked })} />
        </label>
        <label className="row">
          <span>Generate link previews</span>
          <input type="checkbox" checked={s.generateLinkPreviews} onChange={(e) => patch({ generateLinkPreviews: e.target.checked })} />
        </label>
        <label className="row">
          <span>Always paste as plain text</span>
          <input type="checkbox" checked={s.pasteAsPlainText} onChange={(e) => patch({ pasteAsPlainText: e.target.checked })} />
        </label>
        <label className="row">
          <span>Paste stack direction</span>
          <select value={s.stackDirection} onChange={(e) => patch({ stackDirection: e.target.value as "fifo" | "lifo" })}>
            <option value="fifo">Queue (first in, first out)</option>
            <option value="lifo">Stack (last in, first out)</option>
          </select>
        </label>
      </section>

      <section>
        <h2>Excluded apps</h2>
        <textarea
          rows={4}
          placeholder="One app name per line (history from these apps is ignored)"
          value={s.excludedApps.join("\n")}
          onChange={(e) => patch({ excludedApps: e.target.value.split("\n").map((x) => x.trim()).filter(Boolean) })}
        />
      </section>

      <section>
        <h2>Danger zone</h2>
        <button
          className="danger"
          onClick={() => {
            if (window.confirm("Clear all non-pinned history?")) void window.clipdeck.clearHistory();
          }}
        >
          Clear history
        </button>
      </section>
    </div>
  );
}
