import { getCurrentWindow } from "@tauri-apps/api/window";
import { wireSettings } from "./settings";
import { wireModules } from "./modules";
import { wireFontSize } from "./ui-font";
import { loadUi } from "./ui-store";
import { hydratePinned, wirePin } from "./pin";
import { closeTitleMenu, wireTitlebarMenu } from "./title_menu";
import { requestMinimize } from "./launch_hide";

const toastEl = document.getElementById("toast") as HTMLElement;
const clockEl = document.getElementById("clock") as HTMLElement;
const win = getCurrentWindow();

let toastTimer = 0;

function toast(msg: string, ms = 2800) {
  toastEl.textContent = msg;
  toastEl.hidden = false;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toastEl.hidden = true;
  }, ms);
}

document.getElementById("btn-min")!.addEventListener("click", () => {
  void requestMinimize();
});
document.getElementById("btn-close")!.addEventListener("click", () => win.close());
wirePin();

function copyStatus(): string {
  const clock = clockEl.textContent?.trim() ?? "";
  const mt = (document.getElementById("mt-dst") as HTMLTextAreaElement | null)?.value.trim() ?? "";
  const chat = (document.getElementById("chat-status") as HTMLElement | null)?.textContent?.trim() ?? "";
  return [clock, mt, chat].filter(Boolean).join("\n");
}

document.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape") {
    closeTitleMenu();
    const settings = document.getElementById("settings");
    if (settings && !settings.hidden) settings.hidden = true;
  }
});

const MONTHS = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

function tickClock() {
  const d = new Date();
  const utc = d.toISOString().slice(11, 16);
  const local = d.toTimeString().slice(0, 8);
  const date = `${String(d.getDate()).padStart(2, "0")}${MONTHS[d.getMonth()]}${String(d.getFullYear()).slice(-2)}`;
  clockEl.textContent = `[${utc}]  ${local}  ${date}`;
}
tickClock();
window.setInterval(tickClock, 1000);

document.querySelectorAll<HTMLButtonElement>("#grips [data-resize]").forEach((grip) => {
  grip.addEventListener("mousedown", (ev) => {
    ev.preventDefault();
    const dir = grip.dataset.resize;
    if (!dir) return;
    void win.startResizeDragging(dir as Parameters<typeof win.startResizeDragging>[0]);
  });
});

wireSettings(toast);
wireFontSize(toast);
wireModules(toast);
wireTitlebarMenu({ toast, copyStatus, closeRadial: () => {} });
loadUi()
  .then((ui) => {
    hydratePinned(ui.pinned !== false);
  })
  .catch((err) => toast(String(err)));
