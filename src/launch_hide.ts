import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { saveUi, uiCache } from "./ui-store";

export type LaunchHide = "off" | "min" | "tray";

export function normalizeLaunchHide(s: string | undefined): LaunchHide {
  return s === "min" || s === "tray" ? s : "off";
}

export function paintLaunchHide(mode: LaunchHide) {
  const min = document.getElementById("set-launch-min") as HTMLInputElement | null;
  const tray = document.getElementById("set-launch-tray") as HTMLInputElement | null;
  if (min) min.checked = mode === "min";
  if (tray) tray.checked = mode === "tray";
}

export function readLaunchHide(): LaunchHide {
  const min = document.getElementById("set-launch-min") as HTMLInputElement | null;
  const tray = document.getElementById("set-launch-tray") as HTMLInputElement | null;
  if (tray?.checked) return "tray";
  if (min?.checked) return "min";
  return "off";
}

export async function requestMinimize(): Promise<void> {
  const mode = normalizeLaunchHide(uiCache()?.launch_hide);
  if (mode === "tray") {
    await invoke("hide_to_tray");
    return;
  }
  await getCurrentWindow().minimize();
}

export function wireLaunchHide(onErr: (e: unknown) => void) {
  const min = document.getElementById("set-launch-min") as HTMLInputElement | null;
  const tray = document.getElementById("set-launch-tray") as HTMLInputElement | null;
  if (!min || !tray) return;

  async function persist(mode: LaunchHide) {
    paintLaunchHide(mode);
    const ui = uiCache();
    if (ui) {
      ui.launch_hide = mode;
      try {
        await saveUi();
      } catch (e) {
        onErr(e);
      }
    }
    try {
      await invoke("set_launch_hide", { mode });
    } catch (e) {
      onErr(e);
    }
  }

  min.addEventListener("change", () => {
    if (min.checked) tray.checked = false;
    void persist(readLaunchHide());
  });
  tray.addEventListener("change", () => {
    if (tray.checked) min.checked = false;
    void persist(readLaunchHide());
  });
}
