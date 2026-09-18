import { getCurrentWindow } from "@tauri-apps/api/window";
import { loadUi, saveUi, uiCache } from "./ui-store";

let pinned = true;

function updatePinChrome(on: boolean) {
  const pinBtn = document.getElementById("btn-pin");
  pinBtn?.classList.toggle("on", on);
  pinBtn?.setAttribute("aria-pressed", String(on));
  document.getElementById("set-pin")?.classList.toggle("on", on);
  const menuPin = document.getElementById("title-menu-pin");
  menuPin?.classList.toggle("on", on);
  menuPin?.setAttribute("aria-checked", String(on));
}

export function isPinned(): boolean {
  return pinned;
}

/** Sync chrome (and the Tauri flag) without writing ui.json — used on load. */
export function hydratePinned(on: boolean) {
  pinned = on;
  updatePinChrome(on);
  void getCurrentWindow()
    .setAlwaysOnTop(on)
    .catch(() => {
      /* Vite preview has no window plugin */
    });
}

export async function setPinned(on: boolean): Promise<void> {
  hydratePinned(on);
  try {
    const ui = uiCache() ?? (await loadUi());
    ui.pinned = on;
    await saveUi();
  } catch {
    /* Vite preview has no get_ui / save_ui */
  }
}

export function wirePin() {
  document.getElementById("btn-pin")?.addEventListener("click", () => {
    void setPinned(!isPinned());
  });
}
