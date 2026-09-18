import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AUTO_LLM_AFTER_LEX,
  DEFAULT_USE_OPUS,
  followShouldRerunMt,
  formatLexFallback,
  formatLexStatus,
  formatLlmFailure,
  formatOpusProgress,
  formatOpusStatus,
  formatTranslateError,
  mergeQueuedLlm,
} from "./mt_policy";
import { applyFontPx, currentFontPx, FONT_PX_DEFAULT } from "./ui-font";
import { loadUi, saveUi, uiCache } from "./ui-store";

type WindowInfo = { title: string; hwnd: number };
type ToastFn = (msg: string, ms?: number) => void;

const MODS = ["translate", "ocr", "chat"];

export function wireModules(toast: ToastFn) {
  const modules = document.getElementById("modules") as HTMLElement;
  const mtFrom = document.getElementById("mt-from") as HTMLSelectElement;
  const mtTo = document.getElementById("mt-to") as HTMLSelectElement;
  const mtSrc = document.getElementById("mt-src") as HTMLTextAreaElement;
  const mtDst = document.getElementById("mt-dst") as HTMLTextAreaElement;
  const mtStatus = document.getElementById("mt-status") as HTMLElement;
  const ocrWin = document.getElementById("ocr-win") as HTMLSelectElement;
  const ocrOut = document.getElementById("ocr-out") as HTMLTextAreaElement;
  const ocrStatus = document.getElementById("ocr-status") as HTMLElement;
  const chatEngine = document.getElementById("chat-engine") as HTMLSelectElement;
  const chatModel = document.getElementById("chat-model") as HTMLSelectElement;
  const chatLog = document.getElementById("chat-log") as HTMLElement;
  const chatIn = document.getElementById("chat-in") as HTMLInputElement;
  const chatStatus = document.getElementById("chat-status") as HTMLElement;
  // `backend` is storage-only: Rust's wire type is role+content, so serde drops
  // this tag before either provider sees a request body.
  type ChatMsg = { role: string; content: string; backend?: string };
  type ChatLog = { engine: string; model: string; messages: ChatMsg[] };
  let messages: ChatMsg[] = [];
  let mtTimer = 0;
  let mtInFlight = false;
  let mtQueued = false;
  let mtQueuedLlm = false;
  let mtEpoch = 0;
  let mtScrollLock = false;
  let mtFromPaste = false;
  let followTimer = 0;
  let followOn = false;
  let followLast = "";
  let followBusy = false;

  type TranslateOut = {
    text: string;
    cached: boolean;
    engine: string;
    model?: string | null;
    warning?: string | null;
  };

  function setMtStatus(text: string, kind: "busy" | "ok" | "err" | "") {
    mtStatus.textContent = text;
    mtStatus.classList.toggle("busy", kind === "busy");
    mtStatus.classList.toggle("ok", kind === "ok");
    mtStatus.classList.toggle("err", kind === "err");
  }

  void listen<string>("mt-progress", (ev) => {
    const msg = (ev.payload || "").trim();
    if (msg) setMtStatus(formatOpusProgress(msg), "busy");
  });

  function showMod(id: string) {
    const next = MODS.includes(id) ? id : "translate";
    for (const pane of modules.querySelectorAll<HTMLElement>(".mod-pane")) {
      pane.hidden = pane.id !== `mod-${next}`;
    }
    for (const btn of modules.querySelectorAll<HTMLButtonElement>("[data-mod]")) {
      btn.classList.toggle("on", btn.dataset.mod === next);
    }
    const ui = uiCache();
    if (ui) {
      ui.last_module = next;
      void persist();
    }
  }

  function pickLang(value: string, fallback: string) {
    return value === "en" || value === "es" ? value : fallback;
  }

  function swapLangSelects() {
    const from = pickLang(mtFrom.value, "es");
    const to = pickLang(mtTo.value, "en");
    mtFrom.value = to;
    mtTo.value = from;
  }

  function syncMtScroll(from: HTMLTextAreaElement, to: HTMLTextAreaElement) {
    if (mtScrollLock) return;
    mtScrollLock = true;
    to.scrollTop = from.scrollTop;
    to.scrollLeft = from.scrollLeft;
    requestAnimationFrame(() => {
      mtScrollLock = false;
    });
  }

  async function maybeSwapWrongPaste(text: string): Promise<string> {
    const from = pickLang(mtFrom.value, "es");
    let detected: string | null = null;
    try {
      detected = await invoke<string | null>("detect_mt_lang", { text });
    } catch {
      return "";
    }
    if (detected !== "es" && detected !== "en") return "";
    if (detected === from) return "";
    swapLangSelects();
    void persist();
    return `${pickLang(mtFrom.value, "es")}→${pickLang(mtTo.value, "en")}`;
  }

  const BACKEND_LABEL: Record<string, string> = { ollama: "ollama", dsf: "dsf" };

  function appendBubble(m: ChatMsg) {
    const el = document.createElement("div");
    el.className = m.role === "user" ? "me" : "bot";
    // Rows restored from before this tag existed have no backend; say nothing
    // rather than guess which engine answered them.
    const label = BACKEND_LABEL[m.backend ?? ""];
    if (label) {
      const tag = document.createElement("span");
      tag.className = `who ${m.backend}`;
      tag.textContent = label;
      el.appendChild(tag);
    }
    el.appendChild(document.createTextNode(m.content));
    chatLog.appendChild(el);
  }

  async function saveChat() {
    try {
      await invoke("chat_save", {
        log: { engine: engine(), model: chatModel.value, messages },
      });
    } catch (e) {
      chatStatus.textContent = String(e);
    }
  }

  async function restoreChat() {
    try {
      const log = await invoke<ChatLog>("chat_load");
      messages = Array.isArray(log.messages) ? log.messages : [];
      chatLog.replaceChildren();
      for (const m of messages) appendBubble(m);
      chatLog.scrollTop = chatLog.scrollHeight;
      if (messages.length) chatStatus.textContent = `${messages.length} restored`;
    } catch (e) {
      chatStatus.textContent = String(e);
    }
  }

  function engine(): "ollama" | "dsf" {
    return chatEngine.value === "dsf" ? "dsf" : "ollama";
  }

  function chatError(err: unknown): string {
    return engine() === "dsf" ? String(err) : ollamaError(err);
  }

  function ollamaHostLabel(url: string): string {
    try {
      const u = new URL(url);
      return u.port ? `${u.hostname}:${u.port}` : u.hostname;
    } catch {
      return url.replace(/^https?:\/\//i, "").replace(/\/$/, "") || "127.0.0.1:11434";
    }
  }

  function ollamaDownMsg(): string {
    const url = uiCache()?.ollama_url || "http://127.0.0.1:11434";
    return `Ollama not running on ${ollamaHostLabel(url)}`;
  }

  function ollamaError(err: unknown): string {
    const s = String(err);
    if (/^Ollama not running on /i.test(s)) return s;
    if (
      /connection refused|actively refused|tcp connect|error sending request|timed out|timeout|connect error|11434/i.test(
        s,
      )
    ) {
      return ollamaDownMsg();
    }
    return s;
  }

  async function persist() {
    const ui = uiCache();
    if (!ui) return;
    ui.mt_from = pickLang(mtFrom.value, "es");
    ui.mt_to = pickLang(mtTo.value, "en");
    ui.mt_enrich = false;
    ui.chat_engine = engine();
    if (engine() === "ollama") ui.ollama_model = chatModel.value;
    ui.wa_title = ocrWin.value || ui.wa_title;
    ui.font_px = currentFontPx();
    try {
      await saveUi();
    } catch (e) {
      toast(String(e));
    }
  }

  async function loadWindows() {
    try {
      const list = await invoke<WindowInfo[]>("list_app_windows");
      const prev = uiCache()?.wa_title || "WhatsApp";
      ocrWin.replaceChildren();
      const seen = new Set<string>();
      for (const w of list) {
        if (seen.has(w.title)) continue;
        seen.add(w.title);
        const opt = document.createElement("option");
        opt.value = w.title;
        opt.textContent = w.title.slice(0, 80);
        ocrWin.appendChild(opt);
      }
      const hit = [...ocrWin.options].find(
        (o) => o.value.toLowerCase().includes(prev.toLowerCase()) || o.value === prev,
      );
      if (hit) ocrWin.value = hit.value;
      else if (prev) {
        const opt = document.createElement("option");
        opt.value = prev;
        opt.textContent = prev;
        ocrWin.insertBefore(opt, ocrWin.firstChild);
        ocrWin.value = prev;
      }
    } catch (e) {
      ocrStatus.textContent = String(e);
    }
  }

  function fillModels(names: string[]) {
    chatModel.replaceChildren();
    for (const n of names) {
      const opt = document.createElement("option");
      opt.value = n;
      opt.textContent = n;
      chatModel.appendChild(opt);
    }
  }

  async function loadModels() {
    chatStatus.textContent = "…";
    if (engine() === "dsf") {
      try {
        fillModels(await invoke<string[]>("dsf_models"));
        chatStatus.textContent = await invoke<string>("dsf_ping");
      } catch (e) {
        chatStatus.textContent = String(e);
      }
      return;
    }
    try {
      const names = await invoke<string[]>("ollama_models");
      fillModels(names);
      const savedModel = uiCache()?.ollama_model;
      if (savedModel && names.includes(savedModel)) chatModel.value = savedModel;
      chatStatus.textContent = names.length ? `${names.length} models` : "no models";
    } catch (e) {
      chatStatus.textContent = ollamaError(e);
    }
  }

  async function runTranslate(opus = DEFAULT_USE_OPUS) {
    if (AUTO_LLM_AFTER_LEX) {
      throw new Error("auto LLM after lex is forbidden");
    }
    if (mtInFlight) {
      mtQueued = true;
      mtQueuedLlm = mergeQueuedLlm(mtQueuedLlm, opus);
      return;
    }
    mtInFlight = true;
    let useOpus = opus;
    try {
      do {
        if (mtQueued) {
          useOpus = mtQueuedLlm;
        }
        mtQueued = false;
        mtQueuedLlm = false;
        const epoch = mtEpoch;
        const source = mtSrc.value;
        if (!source.trim()) {
          mtDst.value = "";
          setMtStatus("", "");
          break;
        }
        const swappedTo = mtFromPaste ? await maybeSwapWrongPaste(source) : "";
        mtFromPaste = false;
        if (epoch !== mtEpoch) break;
        let from = pickLang(mtFrom.value, "es");
        let to = pickLang(mtTo.value, "en");
        if (from === to) {
          from = "es";
          to = "en";
          mtFrom.value = "es";
          mtTo.value = "en";
        }
        setMtStatus(
          useOpus ? (followOn ? "follow · opus…" : "opus…") : formatLexStatus(followOn),
          "busy",
        );
        try {
          const out = await invoke<TranslateOut>("translate_text", {
            source,
            from,
            to,
            llm: useOpus,
          });
          if (epoch !== mtEpoch) break;
          if (mtQueued) continue;
          if (
            mtSrc.value !== source ||
            pickLang(mtFrom.value, "es") !== from ||
            pickLang(mtTo.value, "en") !== to
          ) {
            continue;
          }
          mtDst.value = out.text;
          syncMtScroll(mtSrc, mtDst);
          if (useOpus && out.engine === "lex") {
            setMtStatus(formatLexFallback(out.warning || "Opus-MT unavailable", followOn), "err");
          } else if (useOpus) {
            setMtStatus(
              swappedTo ? `swapped to ${swappedTo}` : formatOpusStatus(out, followOn),
              "ok",
            );
          } else {
            setMtStatus(swappedTo ? `swapped to ${swappedTo}` : formatLexStatus(followOn), "ok");
          }
        } catch (e) {
          if (epoch !== mtEpoch) break;
          if (mtQueued) continue;
          if (
            mtSrc.value !== source ||
            pickLang(mtFrom.value, "es") !== from ||
            pickLang(mtTo.value, "en") !== to
          ) {
            continue;
          }
          if (useOpus) {
            setMtStatus(formatLlmFailure(e), "err");
          } else {
            mtDst.value = "";
            setMtStatus(formatTranslateError(e), "err");
          }
        }
      } while (mtQueued);
    } finally {
      mtInFlight = false;
      if (mtQueued) {
        const again = mtQueuedLlm;
        mtQueued = false;
        mtQueuedLlm = false;
        void runTranslate(again);
      }
    }
  }

  function scheduleTranslate() {
    window.clearTimeout(mtTimer);
    mtTimer = window.setTimeout(() => void runTranslate(), 450);
  }

  modules.querySelectorAll<HTMLButtonElement>("[data-mod]").forEach((btn) => {
    btn.addEventListener("click", () => showMod(btn.dataset.mod || "translate"));
  });
  mtSrc.addEventListener("scroll", () => syncMtScroll(mtSrc, mtDst));
  mtDst.addEventListener("scroll", () => syncMtScroll(mtDst, mtSrc));
  document.getElementById("mt-swap")!.addEventListener("click", () => {
    swapLangSelects();
    const src = mtSrc.value;
    mtSrc.value = mtDst.value;
    mtDst.value = src;
    syncMtScroll(mtSrc, mtDst);
    void persist();
    void runTranslate();
  });
  document.getElementById("mt-copy")!.addEventListener("click", async () => {
    const text = mtDst.value;
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setMtStatus("copied", "ok");
    } catch (e) {
      toast(String(e));
    }
  });
  document.getElementById("mt-clear")!.addEventListener("click", () => {
    mtEpoch += 1;
    mtQueued = false;
    mtQueuedLlm = false;
    stopFollow();
    mtSrc.value = "";
    mtDst.value = "";
    setMtStatus("", "");
  });

  const mtFollow = document.getElementById("mt-follow") as HTMLButtonElement;

  function setFollowUi(on: boolean) {
    followOn = on;
    mtFollow.classList.toggle("on", on);
    mtFollow.setAttribute("aria-pressed", on ? "true" : "false");
  }

  function stopFollow() {
    window.clearInterval(followTimer);
    followTimer = 0;
    followLast = "";
    setFollowUi(false);
  }

  async function followTick() {
    if (!followOn || followBusy) return;
    followBusy = true;
    try {
      const out = await invoke<{ text: string; source: string }>("capture_ocr", {
        title: ocrWin.value,
      });
      const text = out.text.trim();
      if (!followShouldRerunMt(followLast, text, mtInFlight)) {
        if (!text) {
          setMtStatus("follow · no text", "err");
          return;
        }
        const cur = mtStatus.textContent || "";
        if (!cur.includes("opus") && !cur.includes("getting ") && !cur.includes("loading ")) {
          setMtStatus("follow · opus", "busy");
        }
        return;
      }
      followLast = text;
      mtSrc.value = text;
      void runTranslate(true);
    } catch (e) {
      setMtStatus(String(e), "err");
    } finally {
      followBusy = false;
    }
  }

  document.getElementById("mt-go")!.addEventListener("click", () => {
    void runTranslate(true);
  });
  document.getElementById("mt-llm")!.addEventListener("click", () => {
    void runTranslate(true);
  });

  mtFollow.addEventListener("click", () => {
    if (followOn) {
      stopFollow();
      return;
    }
    setFollowUi(true);
    showMod("translate");
    setMtStatus("follow · opus…", "busy");
    void followTick();
    followTimer = window.setInterval(() => void followTick(), 700);
  });
  mtFrom.addEventListener("change", () => {
    void persist();
    void runTranslate();
  });
  mtTo.addEventListener("change", () => {
    void persist();
    void runTranslate();
  });
  mtSrc.addEventListener("paste", () => {
    mtFromPaste = true;
    window.clearTimeout(mtTimer);
  });
  mtSrc.addEventListener("input", () => {
    if (mtFromPaste) {
      void runTranslate();
      return;
    }
    scheduleTranslate();
  });
  mtSrc.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter" && (ev.ctrlKey || ev.metaKey)) {
      ev.preventDefault();
      void runTranslate();
    }
  });
  document.getElementById("ocr-bind")!.addEventListener("click", async () => {
    await loadWindows();
    const ui = uiCache();
    if (ui && ocrWin.value) {
      ui.wa_title = ocrWin.value;
      await persist();
      ocrStatus.textContent = `bound ${ocrWin.value.slice(0, 40)}`;
    }
  });
  document.getElementById("ocr-go")!.addEventListener("click", async () => {
    ocrStatus.textContent = "ocr…";
    try {
      await persist();
      const out = await invoke<{ text: string; source: string }>("capture_ocr", {
        title: ocrWin.value,
      });
      ocrOut.value = out.text;
      const src = out.source.trim();
      ocrStatus.textContent = out.text.trim()
        ? src
          ? `ok · ${src.slice(0, 36)}`
          : "ok"
        : "empty";
    } catch (e) {
      ocrStatus.textContent = String(e);
    }
  });
  document.getElementById("ocr-to-mt")!.addEventListener("click", () => {
    const text = ocrOut.value.trim();
    if (!text) {
      ocrStatus.textContent = "nothing to translate";
      return;
    }
    mtSrc.value = text;
    showMod("translate");
    void runTranslate();
  });
  document.getElementById("chat-refresh")!.addEventListener("click", () => void loadModels());
  document.getElementById("chat-send")!.addEventListener("click", async () => {
    const text = chatIn.value.trim();
    if (!text) return;
    chatIn.value = "";
    // Pin the backend for this turn. Reading engine() again after the round trip
    // would mislabel the reply if the picker moved while it was in flight.
    const backend = engine();
    const sent: ChatMsg = { role: "user", content: text, backend };
    messages.push(sent);
    appendBubble(sent);
    chatLog.scrollTop = chatLog.scrollHeight;
    chatStatus.textContent = "…";
    // Save the question before the round trip so a crash or a kill mid-reply
    // still leaves it on disk.
    await saveChat();
    try {
      await persist();
      const reply = await invoke<string>(
        backend === "dsf" ? "dsf_chat" : "ollama_chat",
        { model: chatModel.value, messages },
      );
      const got: ChatMsg = { role: "assistant", content: reply, backend };
      messages.push(got);
      appendBubble(got);
      chatLog.scrollTop = chatLog.scrollHeight;
      chatStatus.textContent = "ok";
      await saveChat();
    } catch (e) {
      chatStatus.textContent = chatError(e);
    }
  });
  chatEngine.addEventListener("change", async () => {
    await persist();
    await loadModels();
  });
  chatIn.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") document.getElementById("chat-send")!.click();
  });

  loadUi()
    .then(async (s) => {
      applyFontPx(s.font_px ?? FONT_PX_DEFAULT);
      mtFrom.value = pickLang(s.mt_from, "es");
      mtTo.value = pickLang(s.mt_to, "en");
      if (mtFrom.value === mtTo.value) {
        mtFrom.value = "es";
        mtTo.value = "en";
      }
      chatEngine.value = s.chat_engine === "dsf" ? "dsf" : "ollama";
      showMod(s.last_module);
      await loadWindows();
      await restoreChat();
      await loadModels();
    })
    .catch((e) => toast(String(e)));
}
