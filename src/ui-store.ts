import { invoke } from "@tauri-apps/api/core";

export type UiState = {
  last_module: string;
  wa_title: string;
  mt_from: string;
  mt_to: string;
  mt_enrich: boolean;
  ollama_model: string;
  ollama_url: string;
  chat_engine: string;
  font_px?: number;
  win_mode?: string;
  pinned?: boolean;
};

let cache: UiState | null = null;
let loading: Promise<UiState> | null = null;

function normalize(s: UiState): UiState {
  return {
    ...s,
    chat_engine: s.chat_engine === "dsf" ? "dsf" : "ollama",
    pinned: s.pinned !== false,
  };
}

export function uiCache(): UiState | null {
  return cache;
}

export function loadUi(): Promise<UiState> {
  if (!loading) {
    loading = invoke<UiState>("get_ui").then((s) => {
      cache = normalize(s);
      return cache;
    });
  }
  return loading;
}

export async function saveUi(): Promise<void> {
  if (!cache) return;
  await invoke("save_ui", { ui: cache });
}
