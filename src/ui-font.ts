import { invoke } from "@tauri-apps/api/core";

export const FONT_PX_MIN = 11;
export const FONT_PX_MAX = 22;
export const FONT_PX_DEFAULT = 13;

type ToastFn = (msg: string, ms?: number) => void;

let fontPx = FONT_PX_DEFAULT;

export function clampFontPx(px: number): number {
  if (!Number.isFinite(px)) return FONT_PX_DEFAULT;
  return Math.min(FONT_PX_MAX, Math.max(FONT_PX_MIN, Math.round(px)));
}

export function currentFontPx(): number {
  return fontPx;
}

export function applyFontPx(px: number): number {
  fontPx = clampFontPx(px);
  document.documentElement.style.setProperty("--ui-font", `${fontPx}px`);
  return fontPx;
}

async function persistFontPx(px: number): Promise<number> {
  const next = applyFontPx(px);
  return invoke<number>("set_font_px", { px: next });
}

export async function nudgeFontPx(delta: number, toast?: ToastFn): Promise<number> {
  const next = clampFontPx(fontPx + delta);
  if (next === fontPx) return fontPx;
  applyFontPx(next);
  toast?.(`${next}px`, 800);
  try {
    return await persistFontPx(next);
  } catch (e) {
    toast?.(String(e));
    return next;
  }
}

function isFontHotkey(ev: KeyboardEvent): "inc" | "dec" | "reset" | null {
  if (!(ev.ctrlKey || ev.metaKey) || ev.altKey) return null;
  if (ev.key === "+" || ev.key === "=" || ev.code === "Equal" || ev.code === "NumpadAdd") {
    return "inc";
  }
  if (ev.key === "-" || ev.key === "_" || ev.code === "Minus" || ev.code === "NumpadSubtract") {
    return "dec";
  }
  if (ev.key === "0") return "reset";
  return null;
}

export function wireFontSize(toast: ToastFn) {
  invoke<{ font_px?: number }>("get_ui")
    .then((s) => applyFontPx(s.font_px ?? FONT_PX_DEFAULT))
    .catch((e) => toast(String(e)));

  window.addEventListener(
    "keydown",
    (ev) => {
      const op = isFontHotkey(ev);
      if (!op) return;
      ev.preventDefault();
      ev.stopPropagation();
      const next =
        op === "reset" ? FONT_PX_DEFAULT : clampFontPx(fontPx + (op === "inc" ? 1 : -1));
      if (next === fontPx) return;
      applyFontPx(next);
      toast(`${next}px`, 800);
      void persistFontPx(next).catch((e) => toast(String(e)));
    },
    true,
  );
}
