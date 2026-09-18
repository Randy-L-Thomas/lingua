/** Frozen Translate contract: Opus-MT CPU by default. Never overlay Ollama after lex. */
export const AUTO_LLM_AFTER_LEX = false;

/** Go / Follow / paste / type / lang send llm: true (Opus-MT). */
export const DEFAULT_USE_OPUS = true;

/** Later Opus request must survive a lex fallback retry. Lex cannot downgrade a queued Opus run. */
export function mergeQueuedLlm(queuedLlm: boolean, incomingLlm: boolean): boolean {
  return queuedLlm || incomingLlm;
}

/** Follow must not re-run Opus when OCR text is unchanged or a run is already in flight. */
export function followShouldRerunMt(
  prev: string,
  next: string,
  inFlight: boolean,
): boolean {
  if (inFlight) return false;
  const text = next.trim();
  return text.length > 0 && text !== prev;
}

export function formatLexStatus(follow: boolean): string {
  return follow ? "follow · lex" : "lex";
}

export function formatLexFallback(err: unknown, follow: boolean): string {
  const prefix = follow ? "follow · lex fallback" : "lex fallback";
  return `${prefix} · ${formatTranslateError(err)}`;
}

export function formatOpusStatus(
  out: {
    engine: string;
    cached: boolean;
    model?: string | null;
  },
  follow = false,
): string {
  if (out.engine === "same") return follow ? "follow · same" : "opus · same";
  const model = out.model?.trim();
  let core = "opus";
  if (out.cached) core = model ? `opus · cache · ${model}` : "opus · cache";
  else if (model) core = `opus · ${model}`;
  return follow ? `follow · ${core}` : core;
}

export function formatOpusProgress(msg: string): string {
  const t = msg.trim();
  if (!t) return "opus…";
  if (t.startsWith("opus") || t.startsWith("getting ") || t.startsWith("loading ")) return t;
  return `opus · ${t}`;
}

/** Translate/Opus errors must not be rewritten as Ollama. */
export function formatTranslateError(err: unknown): string {
  return String(err).replace(/^Error:\s*/, "");
}

export function formatLlmFailure(err: unknown): string {
  return `${formatTranslateError(err)} · dest unchanged`;
}
