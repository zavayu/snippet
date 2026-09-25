# Snippet

> Your local AI companion for the text you are working with.

Snippet is a small Windows desktop assistant for those moments when you need a
quick answer about something on your screen. Highlight text in any app, press a
shortcut, and get a streamed response from a local Ollama model in a lightweight
popup.

It is made for the little questions that come up while you work: explain a
passage, summarize a section, refine a sentence, or ask about the text in front
of you without breaking your flow.

## Features

- **System-wide selection capture** — select text in a browser, editor, or other
  desktop application and press `Ctrl + Shift + Space`.
- **Local Ollama inference** — send prompts to a model running on your machine.
- **Streaming responses** — see the answer as it is generated.
- **Markdown answers** — read headings, lists, tables, links, and code in a
  compact, desktop-native format.
- **Quick actions and custom prompts** — explain, summarize, refine, or provide
  your own instruction.
- **Persistent local settings** — configure the Ollama address and selected model
  once; Snippet remembers them.
- **Compact desktop UI** — borderless, always-on-top, cursor-positioned popup
  that stays out of your taskbar.
- **Safe clipboard handling** — restores prior plain-text clipboard content when
  possible after capturing a selection.

## How it works

```text
Select text in any application
        ↓
Ctrl + Shift + Space
        ↓
Snippet captures the selection
        ↓
PromptBuilder creates chat messages
        ↓
Ollama streams a local response
        ↓
Snippet displays the answer
```

## Requirements

- Windows
- Node.js and a Rust toolchain for development
- [Ollama](https://ollama.com/) installed locally
- At least one installed Ollama chat model

## Getting started

From the project directory:

```powershell
npm install
npm run tauri dev
```

Then:

1. Open Snippet’s Settings gear.
2. Confirm the Ollama address (the default is `http://localhost:11434`).
3. Select an installed model and save.
4. Highlight text in another application and press `Ctrl + Shift + Space`.
5. Choose a quick action or enter an instruction.

If Ollama is reachable but no model is listed, install one with Ollama before
continuing:

```powershell
ollama pull <model-name>
```

## Configuration

Snippet stores its runtime settings in the Windows app-config directory:

```text
%APPDATA%\com.snippet.app\settings.json
```

The file contains the Ollama address and selected model:

```json
{
  "ollamaBaseUrl": "http://localhost:11434",
  "model": "your-model-name",
  "thinking": false
}
```

For development, `SNIPPET_OLLAMA_MODEL` overrides the saved model name without
changing the settings file. Thinking is disabled by default for faster replies;
enable it in Settings when you prefer more deliberate model responses.

### Development tools

Selected-text and generated-prompt previews are disabled by default. To enable
them for your machine, create an ignored `.env.development.local` file:

```env
VITE_SHOW_DEVELOPMENT_TOOLS=true
```

The tracked `.env.development` keeps this flag off by default, and release builds
always exclude these preview controls.

## Architecture

```text
src-tauri/src/
├── selection.rs        Windows clipboard-based text capture
├── prompt.rs           Provider-neutral prompt construction
├── provider/
│   ├── mod.rs          LLM provider contract
│   └── ollama.rs       Ollama HTTP and streaming implementation
├── settings.rs         Persisted local AI configuration
└── lib.rs              Tauri commands, events, and application lifecycle
```

The capture, prompt, and provider layers are intentionally independent. That
keeps the current Ollama integration replaceable and leaves room for future
capture strategies without changing the popup interaction.

## Development

```powershell
# Build the frontend
npm run build

# Run the Rust test suite
cargo test --manifest-path src-tauri/Cargo.toml

# Create a bundled desktop build
npm run tauri build
```

## Privacy

By default, Snippet sends selected text only to Ollama at `localhost`. If you
configure a different Ollama address, that server receives the selected text
instead.

## Contributing

Contributions, issues, and ideas are welcome. Please keep changes focused,
preserve the separation between capture, prompt, and provider layers, and add
tests for backend behavior where practical.
