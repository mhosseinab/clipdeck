import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export interface LoginItemEnv {
  platform: NodeJS.Platform;
  enable: boolean;
  isPackaged: boolean;
  execPath: string;
  appPath: string;
  appImage?: string;
  home?: string;
  setLoginItemSettings?: (openAtLogin: boolean) => void;
}

export function linuxAutostartFile(home: string): string {
  return path.join(home, ".config", "autostart", "clipdeck.desktop");
}

export function desktopEntry(exec: string): string {
  return [
    "[Desktop Entry]",
    "Type=Application",
    "Name=ClipDeck",
    "Comment=Clipboard manager",
    `Exec=${exec}`,
    "X-GNOME-Autostart-enabled=true",
    "Hidden=false",
    "NoDisplay=false",
    "",
  ].join("\n");
}

/** Enable/disable launch-at-login. Linux uses ~/.config/autostart; others use the OS API. */
export function applyLoginItem(env: LoginItemEnv): void {
  const home = env.home ?? os.homedir();
  if (env.platform === "linux") {
    const file = linuxAutostartFile(home);
    if (env.enable) {
      const exe = env.appImage ?? env.execPath;
      const exec =
        env.isPackaged && !env.appImage ? `"${exe}"` : `"${exe}" "${env.appPath}"`;
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, desktopEntry(exec));
    } else {
      fs.rmSync(file, { force: true });
    }
    return;
  }
  env.setLoginItemSettings?.(env.enable);
}
