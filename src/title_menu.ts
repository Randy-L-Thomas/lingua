import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isPinned, setPinned } from "./pin";
import { nudgeFontPx } from "./ui-font";

export type TitleMenuHost = {
  toast: (msg: string, ms?: number) => void;
  copyStatus: () => string;
  closeRadial: () => void;
};

const MENU_PAD = 4;

function menuEl(): HTMLElement {
  return document.getElementById("title-menu") as HTMLElement;
}

export function closeTitleMenu() {
  const menu = menuEl();
  if (!menu || menu.hidden) return;
  menu.hidden = true;
  document.body.classList.remove("title-menu-open");
}

function placeMenu(x: number, y: number) {
  const menu = menuEl();
  menu.hidden = false;
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;
  const r = menu.getBoundingClientRect();
  let left = x;
  let top = y;
  if (left + r.width > window.innerWidth - MENU_PAD) {
    left = window.innerWidth - r.width - MENU_PAD;
  }
  if (top + r.height > window.innerHeight - MENU_PAD) {
    top = window.innerHeight - r.height - MENU_PAD;
  }
  menu.style.left = `${Math.max(MENU_PAD, left)}px`;
  menu.style.top = `${Math.max(MENU_PAD, top)}px`;
}

function openTitleMenu(ev: MouseEvent, host: TitleMenuHost) {
  const menu = menuEl();
  if (!menu) return;
  host.closeRadial();
  document.getElementById("title-menu-pin")?.classList.toggle("on", isPinned());
  document.getElementById("title-menu-pin")?.setAttribute("aria-checked", String(isPinned()));
  document.body.classList.add("title-menu-open");
  placeMenu(ev.clientX, ev.clientY);
}

function isNativeMenuTarget(target: EventTarget | null): boolean {
  const node = target as Node | null;
  const el = node instanceof Element ? node : node?.parentElement;
  return Boolean(el?.closest("textarea, input, [contenteditable]:not([contenteditable='false'])"));
}

async function runMenuAction(act: string, host: TitleMenuHost) {
  const win = getCurrentWindow();
  switch (act) {
    case "pin":
      await setPinned(!isPinned());
      return;
    case "dock": {
      try {
        await invoke("set_width_mode", { mode: "half" });
      } catch (e) {
        host.toast(String(e));
      }
      return;
    }
    case "settings":
      document.getElementById("btn-settings")?.click();
      return;
    case "copy-status": {
      const text = host.copyStatus();
      try {
        await navigator.clipboard.writeText(text);
        host.toast("copied status");
      } catch (e) {
        host.toast(String(e));
      }
      return;
    }
    case "font-up":
      await nudgeFontPx(1, host.toast);
      return;
    case "font-down":
      await nudgeFontPx(-1, host.toast);
      return;
    case "min":
      void win.minimize().catch(() => {});
      return;
    case "close":
      void win.close().catch(() => {});
      return;
    default:
      return;
  }
}

export function wireTitlebarMenu(host: TitleMenuHost) {
  const titlebar = document.querySelector(".titlebar") as HTMLElement | null;
  const menu = menuEl();
  if (!titlebar || !menu) return;
  const win = getCurrentWindow();

  titlebar.addEventListener("pointerdown", (ev) => {
    if (ev.button !== 0) return;
    if (document.body.classList.contains("title-menu-open")) return;
    const t = ev.target as HTMLElement | null;
    if (t?.closest("button, input, textarea, select, a, #title-menu")) return;
    void win.startDragging().catch(() => {
      /* Vite preview cannot drag the window */
    });
  });

  document.addEventListener("contextmenu", (ev) => {
    if (isNativeMenuTarget(ev.target)) return;
    ev.preventDefault();
    const t = ev.target as HTMLElement | null;
    if (t?.closest(".titlebar, #title-menu")) {
      openTitleMenu(ev, host);
    }
  });

  menu.addEventListener("click", (ev) => {
    const btn = (ev.target as HTMLElement).closest<HTMLButtonElement>("[data-act]");
    if (!btn || !menu.contains(btn)) return;
    const act = btn.dataset.act || "";
    closeTitleMenu();
    void runMenuAction(act, host);
  });

  window.addEventListener("pointerdown", (ev) => {
    if (menu.hidden) return;
    const t = ev.target as Node | null;
    if (t && menu.contains(t)) return;
    closeTitleMenu();
  });
}
