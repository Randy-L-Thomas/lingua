import assert from "node:assert/strict";
import { test } from "node:test";
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
} from "./mt_policy.ts";

test("lex path never auto-starts Ollama after a preview", () => {
  assert.equal(AUTO_LLM_AFTER_LEX, false);
});

test("default translate path is Opus-MT, not lex", () => {
  assert.equal(DEFAULT_USE_OPUS, true);
});

test("queued Opus request is not downgraded to lex", () => {
  assert.equal(mergeQueuedLlm(false, true), true);
  assert.equal(mergeQueuedLlm(true, false), true);
});

test("queued lex stays lex until Opus is requested", () => {
  assert.equal(mergeQueuedLlm(false, false), false);
});

test("translate errors stay Opus-MT, never remapped to Ollama", () => {
  const download =
    "Could not download es-en: error sending request for url: tcp connect error";
  assert.equal(formatTranslateError(download), download);
  const refused = formatTranslateError(
    "error sending request for url: tcp connect error: connection refused",
  );
  assert.equal(refused.includes("Ollama"), false);
  assert.equal(refused.includes("11434"), false);
});

test("LLM failure says dest was not updated", () => {
  assert.equal(
    formatLlmFailure("Opus-MT files missing for es-en. Lex result is unchanged."),
    "Opus-MT files missing for es-en. Lex result is unchanged. · dest unchanged",
  );
});

test("status names Opus-MT, not lex, on the default path", () => {
  assert.equal(formatLexStatus(false), "lex");
  assert.equal(formatLexStatus(true), "follow · lex");
  assert.equal(
    formatOpusStatus({ engine: "opus", cached: false, model: "opus-mt-es-en" }),
    "opus · opus-mt-es-en",
  );
  assert.equal(
    formatOpusStatus({ engine: "cache", cached: true, model: "opus-mt-es-en" }, true),
    "follow · opus · cache · opus-mt-es-en",
  );
  assert.equal(formatOpusProgress("getting es-en…"), "getting es-en…");
  assert.equal(formatOpusProgress("loading es-en…"), "loading es-en…");
  assert.equal(
    formatLexFallback("Opus-MT files missing for es-en.", false),
    "lex fallback · Opus-MT files missing for es-en.",
  );
});

test("unchanged OCR does not re-run Opus-MT", () => {
  assert.equal(followShouldRerunMt("Me: hola", "Me: hola", false), false);
  assert.equal(followShouldRerunMt("Me: hola", "Me: gracias", false), true);
  assert.equal(followShouldRerunMt("Me: hola", "Me: gracias", true), false);
  assert.equal(followShouldRerunMt("", "", false), false);
});
