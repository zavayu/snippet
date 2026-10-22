# Development guide

## Prerequisites and commands

Build on Windows with Node.js, npm, Rust, and the Windows build tools required by
Tauri. You also need Ollama and an installed chat model to exercise inference.

From the repository root:

```powershell
npm ci
npm run tauri dev
```

Run the checks before submitting a change:

```powershell
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

`npm run tauri build` creates a desktop bundle. See the [release guide](releases.md)
for updater signing and publication. Tests that depend on installed OCR language
packs are explicitly ignored by default; the Windows OCR smoke test can be run
with `cargo test --manifest-path src-tauri/Cargo.toml recognizes_known_text -- --ignored`.

## Architecture

| Location | Responsibility |
| --- | --- |
| `src/App.tsx` and `src/components/` | Popup, settings, conversation UI, and Tauri events |
| `src/conversation.ts` | Frontend conversation state and streamed turn updates |
| `src/popupPosition.ts` | Selection anchors, monitor boundaries, and display scaling |
| `src-tauri/src/selection.rs` and `selection/` | Accessibility capture, copy fallback, editability checks, and replacement |
| `src-tauri/src/screen.rs` | In-memory monitor capture, region cropping, and image encoding |
| `src-tauri/src/ocr.rs` | Native Windows OCR |
| `src-tauri/src/prompt.rs` | Provider-neutral prompt construction |
| `src-tauri/src/provider/` | Provider contract and Ollama HTTP streaming |
| `src-tauri/src/conversation.rs` | Backend conversation context and turn lifecycle |
| `src-tauri/src/settings.rs` | Persisted settings and shortcut configuration |
| `src-tauri/src/lib.rs` | Tauri commands, shortcuts, events, and application lifecycle |

Keep capture, prompt construction, and inference separate. Native OCR receives
original capture pixels; images encoded for Ollama are limited to a 1,920-pixel
edge. Region capture crops before resizing.

## Local configuration

Runtime preferences are stored in the Windows app-config directory:

```text
%APPDATA%\com.snippet.app\settings.json
```

The UI manages this file. Relevant settings include:

```json
{
  "ollamaBaseUrl": "http://localhost:11434",
  "model": "your-model-name",
  "ocrEngine": "windows",
  "ocrModel": "glm-ocr:latest",
  "ocrNumPredict": 2048,
  "ocrNumCtx": 16384,
  "thinking": false,
  "visionEnabled": false
}
```

This is an excerpt, not a complete settings file. Existing configurations without
an OCR engine choice default to Windows OCR. `SNIPPET_OLLAMA_MODEL` overrides the
saved chat model without changing the file. Thinking is disabled by default and
can be enabled in Settings.

To enable development inspection controls, add an ignored
`.env.development.local` file at the repository root:

```env
VITE_SHOW_DEVELOPMENT_TOOLS=true
```

These controls show selected-text and generated-prompt previews, the capture
source, and Editable / Read-only / Unknown status. They are excluded from
production builds even if the environment flag is enabled.

## Selection capture behavior

The accessibility worker tries the focused UIA TextPattern, the focused legacy
IAccessible bridge, ancestor TextPatterns (up to ten levels), and direct window
IAccessible access. It accepts selection ranges or explicit selection strings;
control names and values are never substituted for selected text.

Popup capture gives accessibility up to 120 ms, followed by a copy fallback of up
to 180 ms when inconclusive. Google Docs proxies receive a 600 ms copy budget.
Explicit action shortcuts and retries use longer capture budgets. These limits
bound the caller's wait; late accessibility results are discarded.

Confirmed empty selections skip Copy. Empty Monaco and Google Docs proxies are
inconclusive because their keyboard input may not represent the visible editor
selection. Selected interface objects suppress Copy. Clipboard polling waits for
nonempty text from a consistent, newly published clipboard state.

Visible UIA rectangles anchor the popup below the last selected line, flipping
above when needed. Placement accounts for monitor work areas and display scale;
dragging the popup releases the selection anchor.

Editability is independent of the text capture source. A clipboard capture can
retain confirmed accessibility metadata, but a writable Google Docs keyboard
proxy is not evidence that the canvas document is editable. Replacement requires
a complete selection, confirmed editability, and a stable focused-control
identity. One-use tokens bind the original window, control, and selected text;
replacement rechecks those conditions before inserting a response.
