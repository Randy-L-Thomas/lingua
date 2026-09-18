# Lingua

Translate, OCR, and local LLM chat for the 1920×440 strip. **Tauri 2.** Sibling to [Pulse](https://github.com/Randy-L-Thomas/pulse) (health tiles stay there).

Lingua docks on the **right** 960×440 of the short panel. Pulse Half keeps the left.

Go / Follow / paste run Helsinki-NLP Opus-MT (es↔en) in-process on CPU — not Ollama. First run may download ~300 MB per direction into `%APPDATA%\lingua\mt\`. If that folder is empty, Lingua reuses `%APPDATA%\pulse\mt\` so you do not download twice. Lexicon is fallback only if those files are missing. Chat talks to Ollama (or DSF) if you start it yourself.

User config is `%APPDATA%\lingua\`. First launch copies translate/chat prefs from Pulse’s `ui.json` if Lingua has none. Chat transcript: `%APPDATA%\lingua\chat.json` (copied from Pulse on first run).

```powershell
cd C:\dev\lingua
npm install
npm run tauri dev
```

Pulse and Lingua use different Vite ports (1420 / 1430), so both can run in dev at once.

## Translate contract

Default / Go / Follow / paste / type / lang change = Opus-MT (`llm: true`). Do not start Ollama for Translate. Lexicon (`llm: false`) is fallback only.
