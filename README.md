# Snippet

Snippet is a lightweight Windows desktop utility for asking a local AI model about text selected in any application.

## Stack

- Tauri v2 and Rust for native desktop integration
- React, TypeScript, Vite, and Tailwind CSS for the popup interface
- Ollama as the initial local inference provider

## Development

```powershell
npm install
npm run tauri dev
```

## Current MVP slice

On Windows, select plain text in another application and press `Ctrl + Shift + Space`.
Snippet sends a standard Copy shortcut, waits for the clipboard update, then shows the
captured text in a compact always-on-top popup. Press `Escape` to hide the window.

The capture layer is isolated behind a `SelectionReader` contract. The initial
`ClipboardCopyReader` can later be supplemented by a UI Automation reader without
changing the hotkey workflow or popup UI.

The Copy reader restores the prior clipboard value when it was plain text and the
clipboard has not changed again. It intentionally does not promise restoration of
all rich clipboard formats yet.

The popup now collects either a custom instruction or a selected quick action
(`Explain`, `Summarize`, or `Refine`). The Rust `PromptBuilder` converts that
intent and the selection into provider-neutral chat messages, ready for the
upcoming Ollama streaming integration.

During development builds, the popup includes a **Preview generated prompt**
control below the selected-text preview. It displays the exact system and user
messages produced by `PromptBuilder`; it is omitted from release builds. The
quick-action controls immediately preview their respective prompts during
development, matching their eventual one-click generation behavior. Use the
**Dev On/Off** control in the popup header to show or hide both previews; the
setting persists locally between development sessions.

## MVP path

1. Register a global shortcut.
2. Capture selected text through the clipboard.
3. Stream an Ollama response into the popup.
4. Add model selection and local settings.
