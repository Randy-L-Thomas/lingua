import { invoke } from "@tauri-apps/api/core";
import { disable as disableAutostart, enable as enableAutostart, isEnabled as autostartEnabled } from "@tauri-apps/plugin-autostart";
import { loadUi, saveUi } from "./ui-store";
import { isPinned, setPinned } from "./pin";
import { normalizeLaunchHide, paintLaunchHide, wireLaunchHide } from "./launch_hide";

export type Settings = {
  height: number;
  width: number;
  monitor_width: number;
  monitor_height: number;
};

type ToastFn = (msg: string, ms?: number) => void;

export function wireSettings(toast: ToastFn) {
  const overlay = document.getElementById("settings") as HTMLElement;
  const openBtn = document.getElementById("btn-settings") as HTMLButtonElement;
  const closeBtn = document.getElementById("btn-settings-close") as HTMLButtonElement;
  const saveBtn = document.getElementById("btn-settings-save") as HTMLButtonElement;
  let snapshot: Settings | null = null;

  async function load() {
    snapshot = await invoke<Settings>("get_settings");
    const s = snapshot;
    (document.getElementById("set-mon-w") as HTMLInputElement).value = String(s.monitor_width);
    (document.getElementById("set-mon-h") as HTMLInputElement).value = String(s.monitor_height);
    (document.getElementById("set-panel-w") as HTMLInputElement).value = String(s.width);
    const ui = await loadUi();
    (document.getElementById("set-ollama-url") as HTMLInputElement).value =
      ui.ollama_url || "http://127.0.0.1:11434";
    const pinOn = isPinned();
    document.getElementById("set-pin")!.classList.toggle("on", pinOn);
    try {
      (document.getElementById("set-autostart") as HTMLInputElement).checked = await autostartEnabled();
    } catch {
      (document.getElementById("set-autostart") as HTMLInputElement).checked = false;
    }
    paintLaunchHide(normalizeLaunchHide(ui.launch_hide));
    const meta = await invoke<{ version: string; config_path: string }>("app_meta");
    (document.getElementById("update-meta") as HTMLElement).textContent =
      `v${meta.version}  ·  ${meta.config_path}`;
  }

  function readForm(): Settings {
    const s = snapshot!;
    s.monitor_width = Number((document.getElementById("set-mon-w") as HTMLInputElement).value) || 1920;
    s.monitor_height = Number((document.getElementById("set-mon-h") as HTMLInputElement).value) || 440;
    s.width = Number((document.getElementById("set-panel-w") as HTMLInputElement).value) || 960;
    return s;
  }

  openBtn.addEventListener("click", async () => {
    overlay.hidden = false;
    try {
      await load();
    } catch (e) {
      toast(String(e));
    }
  });
  closeBtn.addEventListener("click", () => {
    overlay.hidden = true;
  });
  window.addEventListener("pointerdown", (ev) => {
    if (overlay.hidden) return;
    const t = ev.target as Node;
    if (overlay.contains(t) || openBtn.contains(t)) return;
    overlay.hidden = true;
  });
  document.getElementById("set-pin")!.addEventListener("click", () => {
    void setPinned(!isPinned());
  });
  document.getElementById("set-autostart")!.addEventListener("change", async (ev) => {
    const on = (ev.target as HTMLInputElement).checked;
    try {
      if (on) await enableAutostart();
      else await disableAutostart();
    } catch (e) {
      toast(String(e));
      (ev.target as HTMLInputElement).checked = await autostartEnabled().catch(() => !on);
    }
  });
  saveBtn.addEventListener("click", async () => {
    if (!snapshot) return;
    try {
      const msg = await invoke<string>("save_settings", { cfg: readForm() });
      const ui = await loadUi();
      ui.ollama_url =
        (document.getElementById("set-ollama-url") as HTMLInputElement).value.trim() ||
        "http://127.0.0.1:11434";
      await saveUi();
      toast(msg);
      overlay.hidden = true;
    } catch (e) {
      toast(String(e));
    }
  });

  wireLaunchHide((e) => toast(String(e)));
  document.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape" && !overlay.hidden) overlay.hidden = true;
  });
}
