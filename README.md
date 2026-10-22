<div align="center">
  <img src="public/snippet-logo.png" alt="Snippet logo" width="160">
  <h1>
  Snippet
  </h1>
  <h3>
    Your local AI companion for the text you are working with.
  </h3>
</div>



Snippet is a small Windows desktop assistant for those moments when you need a
quick answer about something on your screen. Highlight text in any app, press a
shortcut, and get a streamed response from a local Ollama model in a lightweight
popup.

It is made for the little questions that come up while you work: explain a
passage, summarize a section, refine a sentence, or ask about the text in front
of you without breaking your flow.

<p align="center">
  <img src="docs/assets/snippet-demo.gif" alt="Snippet explaining highlighted text in a Google Doc" width="800">
</p>

## Features

- **System-wide selection capture** — select text in a browser, editor, or other
  desktop application and press `Ctrl + Shift + Space`.
- **Current-monitor capture** — press `Ctrl + Shift + I` to attach a resized
  screenshot of the monitor under your cursor for a vision-capable Ollama model.
- **Region capture** — press `Ctrl + Shift + G`, then drag over the part of the
  screen you want Snippet to analyze.
- **Read screen** — extract text from a capture with Windows OCR, or use an Ollama
  OCR model for Markdown, tables, code, and math before reasoning about it.
- **Local Ollama inference** — send prompts to a model running on your machine.
- **Streaming responses** — see the answer as it is generated.
- **Follow-up conversations** - expand an answer into a conversation while
  retaining the original text or screenshot and previous completed replies.
- **Start over without closing** - clear the conversation and its attachments
  and return to the compact input with one click.
- **Markdown answers** — read headings, lists, tables, links, and code in a
  compact, desktop-native format.
- **Quick actions and custom prompts** — explain, summarize, refine, or provide
  your own instruction.
- **Persistent local settings** — configure the Ollama address and selected model
  once; Snippet remembers them.
- **Compact desktop UI** — borderless, always-on-top, cursor-positioned popup
  that stays out of your taskbar.
- **Runs from the system tray** — close the popup without stopping Snippet; open
  it, change settings, or quit from the tray menu.
- **Optional launch at sign-in** — enable it during first-run setup or later in
  Settings → General to keep shortcuts ready in the background.
- **Clipboard-free selection capture** - reads selected text through Windows UI
  Automation when supported. Other apps use a copy fallback that restores prior
  plain-text clipboard content when possible.

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

## Selection capture

Selection shortcuts first ask the foreground application's Windows accessibility
provider for the selected text. Supporting apps leave the clipboard untouched,
and code indentation and trailing newlines are preserved. Disjoint selected
ranges are separated by a newline, with a combined limit of 20,000 characters.

Accessibility lookup follows Selection Hook's Windows ordering: focused UIA
TextPattern, the focused legacy IAccessible bridge, ancestor TextPatterns in the
control-view tree (up to ten levels), then direct window IAccessible access.
Only selection ranges or an explicit selection string are accepted; a control's
full name or value is never substituted. Selected interface objects suppress Copy.
Development tools label successful legacy reads **IAccessible**.

When UI Automation supplies visible selection rectangles, the popup opens below
the last visible selected line, or above the selection if there is not enough
space below. The anchor is retained while the popup resizes, accounts for display
scaling and monitor edges, and is released when you drag the popup. Clipboard,
legacy, and geometry-free captures retain cursor-based placement.

The open-popup shortcut gives accessibility capture up to 120 ms. If that is
inconclusive, it automatically tries copying for up to another 180 ms before
opening the composer. Confirmed empty selections skip Copy entirely.
Empty and inconclusive attempts open quietly, without a capture error.
When capture is inconclusive, a **Capture selected text** icon near Settings lets
you explicitly retry capture from the original app. The popup uses shorter
capture timeouts than explicit capture retries and action shortcuts.

**Capture selected text** temporarily returns to the original app for a full capture,
then attaches the text without clearing your draft. Explain, Summarize, and
Refine shortcuts also use full capture and report specific failures. These
explicit attempts fall back to copying if accessibility is unavailable or takes
too long. A supported control reporting an empty selection does not trigger Copy.
Capture is cancelled if the foreground window changes.

VS Code's Monaco editor also tries the full accessibility sequence before Copy.
Its empty proxy input is treated as inconclusive, since it may not represent the
actual editor selection. Ordinary text fields can report a confirmed empty range.

Accessibility support varies by application. Elevated applications may remain
inaccessible. The copy fallback preserves only prior plain text, not images,
files, or rich formatting; full clipboard preservation is not yet implemented.

When Windows accessibility confirms the selected text is editable, completed
responses offer **Replace selection** next to Copy. Replacement returns focus to
that application and verifies the original control, selected text, and editability
before inserting the response. Each capture permits one replacement attempt;
recapture after editing the source or attempting replacement. Read-only, unknown,
OCR, and truncated selections keep Copy available without offering Replace.
Development diagnostics also show Editable, Read-only, or Unknown.

## Getting started

From the project directory:

```powershell
npm install
npm run tauri dev
```

Then:

1. Snippet opens its first-run setup screen. Install Ollama if needed, confirm
   the address (the default is `http://localhost:11434`), and select an
   installed model.
2. Choose whether Snippet should launch when you sign in, then finish setup.
3. Highlight text in another application and press `Ctrl + Shift + Space`.
4. Choose a quick action or enter an instruction.

## Conversations

After an answer finishes, choose **Copy response**, **Continue conversation**,
or **New conversation**. Continuing expands the popup and focuses a follow-up
input beneath the transcript. Press Enter to send, or Shift+Enter for a new
line. Each assistant reply has its own copy button.

The transcript scrolls while the input stays available. You can stop a response
and retry it; incomplete turns are marked and excluded from subsequent model
context. The original selected text, OCR result, or screenshot remains available
to follow-ups without capturing or reading it again.

**New conversation** cancels any active response, clears the transcript and
attachments, and returns to **Ask me anything**. Reopening the popup or using a
capture shortcut also starts a fresh conversation. Conversations are kept only
in memory. Long conversations have a size limit; start a new conversation when
Snippet asks you to do so.

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
  "ocrEngine": "windows",
  "ocrModel": "glm-ocr:latest",
  "ocrNumPredict": 2048,
  "ocrNumCtx": 16384,
  "thinking": false,
  "visionEnabled": false
}
```

Enable image input only when the selected Ollama model supports screenshots.
The current-monitor and region-capture shortcuts are configurable alongside the
text-action shortcuts; they default to `Ctrl + Shift + I` and `Ctrl + Shift + G`.
Read screen and Read region are separate shortcuts, defaulting to
`Ctrl + Shift + O` and `Ctrl + Shift + T`. They open Snippet immediately,
extract the capture in the background, and use the result as selected text
once it is ready. A prompt or text action chosen while OCR is running is sent
automatically after extraction completes. Configure any shortcut by clicking
its value in Settings and pressing the desired key combination.
Read screen uses Windows OCR by default, locally and without Ollama. It uses
installed Windows OCR language packs, preferring your Windows language settings.
If no recognizer is available, install an OCR language pack through Windows
Settings > Time & language > Language & region.

Choose Ollama under Settings > Read screen > OCR engine for model-based extraction
of Markdown, tables, and formulas. The saved model defaults to `glm-ocr:latest`;
install it with `ollama pull glm-ocr:latest`. Model selection, output limits, and
context limits appear when Ollama is selected. Switching engines preserves these
settings. Existing configurations without an engine choice default to Windows OCR.
Native OCR reads the original cropped pixels; Ollama images retain the 1,920-pixel
size limit. Empty recognition is reported separately from engine failures.

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

### Running in the background

Snippet has no normal application window after it starts. It remains available
from the Windows system tray and through its configured global shortcuts. The
popup's close button and `Esc` only hide the popup; choose **Quit Snippet** from
the tray menu to stop it completely.

The first-run setup offers **Launch Snippet when I sign in**. You can change the
setting at any time under Settings → General. It starts the hidden tray utility
at sign-in; it does not start Ollama or a model until you ask Snippet to do
something.

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

## Releases

See [the release and updater guide](docs/releases.md) for the one-time signing
secret setup, local builds, and end-to-end update testing.

Public Windows builds are published as an NSIS setup executable on the project's
GitHub Releases page. The installer is intentionally per-user, so it should not
need administrator permission.

The repository's release workflow runs when a `vX.Y.Z` tag is pushed. Before
using it, configure the updater signing secret, update the app version, and
create the tag with the same version. It publishes the installer, a Tauri update
signature, and `latest.json`. The installer has no Authenticode signature; Windows may
show a SmartScreen warning for a new or unsigned project. Download only from the
official GitHub Releases page and verify the published SHA-256 checksum when one
is provided.

### In-app updates

Settings → Updates shows the installed version, available release notes, and
download progress. Automatic checks run daily and can be disabled with **Save
settings**. Choose **Download update**, then **Install and restart** when ready.
The tray menu also offers **Check for updates**. A small dot on the Settings
button indicates an available release. Checks do not send selected text or
screenshots to GitHub.

Downloads are verified against the app's embedded public key. Installation waits
until responses, OCR, and captures finish. The installer closes and restarts
Snippet while preserving its configuration. Releases installed before the
updater was added require one manual upgrade.

## Privacy

By default, Snippet sends selected text only to Ollama at `localhost`. If you
configure a different Ollama address, that server receives the selected text
instead.

## Contributing

Contributions, issues, and ideas are welcome. Please keep changes focused,
preserve the separation between capture, prompt, and provider layers, and add
tests for backend behavior where practical.
