import { useEffect, useState } from "react";
import type { Settings } from "./types";

export default function SettingsPage() {
  const [s, setS] = useState<Settings | null>(null);
  const [status, setStatus] = useState("");

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
        <label className="row">
          <span>Show overlay during screen sharing</span>
          <input
            type="checkbox"
            checked={s.showInScreenShare}
            onChange={(e) => patch({ showInScreenShare: e.target.checked })}
          />
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
        <h2>App rules</h2>
        <label className="column">
          <span>Excluded apps — never save clips from these (one per line)</span>
          <textarea
            rows={4}
            value={s.excludedApps.join("\n")}
            onChange={(e) => patch({ excludedApps: e.target.value.split("\n").map((x) => x.trim()).filter(Boolean) })}
          />
        </label>
        <label className="column">
          <span>Always paste as plain text into these apps (one per line)</span>
          <textarea
            rows={3}
            placeholder="e.g. Terminal"
            value={s.plainTextApps.join("\n")}
            onChange={(e) => patch({ plainTextApps: e.target.value.split("\n").map((x) => x.trim()).filter(Boolean) })}
          />
        </label>
      </section>

      <section>
        <h2>Sync</h2>
        <label className="column">
          <span>
            Sync folder — items are shared via a JSONL file in this directory (e.g. a folder synced by
            Syncthing/Dropbox). Leave empty to disable.
          </span>
          <input
            type="text"
            placeholder="/path/to/shared/folder"
            value={s.syncDirectory}
            onChange={(e) => patch({ syncDirectory: e.target.value })}
          />
        </label>
      </section>

      <section>
        <h2>Scripting hooks</h2>
        <label className="column">
          <span>
            One command per line as <code>event=command</code> where event is <code>copy</code> or{" "}
            <code>paste</code>. The item content is piped to the command's stdin (5s timeout).
          </span>
          <textarea
            rows={3}
            placeholder={"copy=notify-send \"copied\"\npaste=pbcopy > /dev/null"}
            value={s.hooks.map((h) => `${h.event}=${h.command}`).join("\n")}
            onChange={(e) =>
              patch({
                hooks: e.target.value
                  .split("\n")
                  .map((line) => {
                    const eq = line.indexOf("=");
                    if (eq <= 0) return null;
                    const event = line.slice(0, eq).trim();
                    const command = line.slice(eq + 1).trim();
                    if ((event === "copy" || event === "paste") && command) return { event: event as "copy" | "paste", command };
                    return null;
                  })
                  .filter((h): h is { event: "copy" | "paste"; command: string } => h !== null),
              })
            }
          />
        </label>
      </section>

      <section>
        <h2>Data</h2>
        <div className="row">
          <span>Export / import history (JSON)</span>
          <div className="btn-group">
            <button
              onClick={() =>
                void window.clipdeck.exportHistory().then((p) => setStatus(p ? `Exported to ${p}` : "Export cancelled"))
              }
            >
              Export…
            </button>
            <button
              onClick={() =>
                void window.clipdeck.importHistory().then((r) =>
                  setStatus(r ? `Imported ${r.imported}, skipped ${r.skipped}` : "Import cancelled"),
                )
              }
            >
              Import…
            </button>
          </div>
        </div>
        <div className="row">
          <span>Updates</span>
          <button onClick={() => void window.clipdeck.checkUpdates().then(setStatus)}>Check for updates</button>
        </div>
        {status && <div className="status-line">{status}</div>}
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
