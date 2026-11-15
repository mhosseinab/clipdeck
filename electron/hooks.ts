import { execFile } from "node:child_process";
import type { ClipItem, Settings } from "./store";

/** Runs user-defined scripting hooks (T2). The item content is piped to the command's stdin. */
export function runHooks(event: "copy" | "paste", item: ClipItem, settings: Settings): void {
  for (const hook of settings.hooks) {
    if (hook.event !== event || !hook.command.trim()) continue;
    try {
      const child = execFile("/bin/sh", ["-c", hook.command], { timeout: 5000 }, () => {});
      child.stdin?.end(item.content);
    } catch {
      /* hook failures never affect the app */
    }
  }
}
