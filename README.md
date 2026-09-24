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

## MVP path

1. Register a global shortcut.
2. Capture selected text through the clipboard.
3. Stream an Ollama response into the popup.
4. Add model selection and local settings.
