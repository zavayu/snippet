<div align="center">
  <img src="public/snippet-logo.png" alt="Snippet" width="160">
  <h1>Snippet</h1>
  <h3>Local AI for the text on your screen.</h3>
  <p>
    <a href="https://github.com/zavayu/snippet/releases/latest">Download for Windows</a> &middot;
    <a href="#getting-started">Getting started</a> &middot;
    <a href="docs/development.md">Development</a> &middot;
    <a href="CHANGELOG.md">Changelog</a>
  </p>
</div>

Highlight text, press a shortcut, and ask a question. Snippet opens a small popup
beside your work and streams an answer from Ollama. Explain a passage, summarize
a section, refine a draft, or keep the conversation going without switching apps.

<p align="center">
  <img src="docs/assets/snippet-demo.gif?" alt="Snippet explaining selected text in a Google Doc" width="800">
</p>

## What you can do

- **Ask about selected text.** Use Explain, Summarize, Refine, or your own prompt
  across browsers, editors, and other desktop apps.
- **Read text from the screen.** Extract a region or an entire monitor with
  Windows OCR, or choose an Ollama OCR model for richer document formatting.
- **Ask about an image.** Attach a screenshot to a vision-capable Ollama model.
- **Research a topic.** Search the web with You.com, then get an Ollama summary
  with clickable citations and sources.
- **Continue the conversation.** Ask follow-up questions with the original
  selection or capture and previous completed replies as context.
- **Use the answer where you work.** Copy a response, or replace the original
  selection when Snippet can confirm the text is editable.
- **Keep it close.** A compact popup, configurable shortcuts, tray access, and
  optional launch at sign-in keep Snippet ready in the background.

## Getting started

You need **Windows**, [Ollama](https://ollama.com/), and an installed Ollama chat
model. Node.js and Rust are needed only when building from source.

1. Download and run the Windows setup executable from
   [GitHub Releases](https://github.com/zavayu/snippet/releases/latest).
2. Start Ollama and install a chat model if you do not already have one.
3. Open Snippet. In first-run setup, confirm the Ollama address, select your
   installed model, and choose whether to launch Snippet when you sign in.
4. Highlight text in another app and press **Ctrl + Shift + Space**.
5. Choose a quick action or type an instruction.

The default Ollama address is `http://localhost:11434`. You can change it and the
selected model in Settings. Enable image input only for a model that supports it.

Snippet runs from the system tray. **Esc** and the popup's close button hide it;
**Quit Snippet** in the tray menu exits the app.

## Keyboard shortcuts

| Action | Default shortcut |
| --- | --- |
| Open Snippet and capture selected text | `Ctrl + Shift + Space` |
| Summarize selected text | `Ctrl + Shift + S` |
| Explain selected text | `Ctrl + Shift + E` |
| Refine selected text | `Ctrl + Shift + R` |
| Attach a screen image | `Ctrl + Shift + I` |
| Attach a region image | `Ctrl + Shift + G` |
| Read screen text with OCR | `Ctrl + Shift + O` |
| Read region text with OCR | `Ctrl + Shift + T` |

Change shortcuts in **Settings > Shortcuts** by clicking a binding and pressing
a new key combination. Clear a binding to disable that shortcut.

## Screen reading and image input

**Read screen** and **Read region** extract text and attach it as reference
material. If you choose an action while OCR is running, Snippet waits for the
text before sending your request.

Choose an engine under **Settings > Read screen > OCR engine**:

| Engine | Best suited to | Requirements |
| --- | --- | --- |
| **Windows OCR** (default) | Fast extraction of ordinary screen text | An installed Windows OCR language pack |
| **Ollama** | Model-based extraction of Markdown, tables, code, and formulas | Ollama and an OCR-capable model |

Windows OCR runs locally without Ollama and uses your installed recognition
languages, preferring your Windows language settings. If a language pack is
missing, install its OCR support in **Windows Settings > Time & language >
Language & region**. Chat responses still use the selected Ollama chat model.

The Ollama OCR model defaults to `glm-ocr:latest`:

```powershell
ollama pull glm-ocr:latest
```

Model selection and output/context limits appear when Ollama OCR is selected.
Switching engines preserves those settings. Recognition quality and model speed
depend on the capture, model, and hardware.

**Attach screen/region image** sends the image itself to your chat model instead
of extracting text first. Enable image input in Settings and select a model with
vision support to use this workflow.

## Web research

Type a question such as "Research restaurants in Santa Clara" and send it.
A tool-capable Ollama model can request web searches when it needs current facts
or external evidence, then read relevant pages to answer. Ordinary explanations and
writing tasks can answer directly. With selected text, choose **Research** to
require a search. Explain, Summarize, and Refine remain local actions.

Snippet checks the installed model's tool capabilities. Models without confirmed
tool support can still chat, with an on-screen notice that new searches are
unavailable. Research requires a tool-capable model. You can inspect support with
`ollama show YOUR_MODEL` and look for `tools` under Capabilities.

Snippet searches You.com and streams an answer with numbered citations. Sources
appear collapsed after the response finishes; expand them to see the query and
links. Copying a research response includes its source URLs. Follow-ups retain
successful tool exchanges and can reuse evidence or search again, preserving
existing source IDs. Each turn allows up to three searches and three page reads,
rejects repeated tool requests, and has a five-minute deadline. Stop cancels the request; failed or
stopped turns do not enter conversation history.

Web search is **enabled by default**. Disable it in **Settings > Web search** and
save your settings to hide Research and block both searching and webpage reading.
Research uses text; read a screenshot with OCR first or type a question.

Select a URL and ask "Summarize this page" to read it directly without a search.
Snippet extracts the main content as Markdown, preserving headings, lists, links,
and code. Long pages are truncated and marked as partial evidence for the model.
The reader supports public UTF-8 HTML on standard HTTP/HTTPS ports. It does not
run JavaScript, use browser logins, read PDFs, or access local/private networks.
Read failures are returned to the model so it can explain the limitation or try
another source.

Free search needs no account or API key and is subject to You.com's service
limits and availability. For account-based access, save an optional You.com API
key in the same settings page. Keys are stored in **Windows Credential Manager**,
not in the settings file. Saving or removing a key applies immediately. A saved
key uses your You.com account's credits; Snippet never switches between free and
paid access automatically. See [You.com's API documentation](https://you.com/docs/agents/mcp-server)
for current access options and limits.

Only the generated search query goes to You.com; the summary is generated by your
configured Ollama server. The query can contain details from the selected text. Reading a webpage sends
its URL directly to that website, without browser cookies or saved credentials.
Search failures and limits are shown as errors, without substituting an answer
that was not researched. Source excerpts are partial evidence, and a citation
link does not guarantee that the model's interpretation is correct.

Search excerpts can contain cached clock readings or old weather conditions.
Snippet supplies the current UTC and device-local timestamp to the model on every
turn, and can answer time questions from that context. It has no dedicated weather
feed, so weather accuracy still depends on suitable dated evidence. Evidence includes its retrieval timestamp,
which is distinct from a page's publication date and an observation's timestamp.

## Conversations and replacement

After an answer finishes, choose **Continue conversation** to ask a follow-up.
Press **Enter** to send or **Shift + Enter** for a new line. Replies render as
Markdown and have individual copy controls. The send button becomes Stop while
a reply is streaming; stopped or failed turns can be retried.

**Replace selection** appears only when accessibility confirms the source text
is editable and Snippet can identify the original control. Before inserting a
completed response, Snippet restores the source app and rechecks the control,
selected text, and editability. If the selection changed, use Copy or recapture
it. Each capture allows one replacement attempt. Read-only, unknown, OCR, and
truncated selections keep Copy available without offering Replace.

**New conversation** clears the transcript and attachments. Opening Snippet
again with a capture shortcut also starts a fresh conversation. Conversations
stay in memory; long conversations have a size limit.

## Application compatibility

Snippet first reads the selection through Windows accessibility. Where supported,
the clipboard stays untouched and the popup opens beneath the selected text.
Otherwise, it falls back to Copy and positions the popup near the cursor.

- **VS Code and Monaco:** empty editor proxies are treated as inconclusive so
  Snippet can try Copy rather than assume nothing is selected.
- **Google Docs:** Snippet recognizes its canvas editor proxies and allows a
  longer copy wait. If your browser does not expose those markers, enabling
  **Tools > Accessibility settings > Turn on screen reader support** in Docs
  may improve capture. Proxy detection alone does not enable Replace.
- **Capture misses:** use **Capture selected text** near Settings to retry from
  the original app without clearing your draft.
- **Protected or elevated apps:** capture may be unavailable. Selections of
  interface objects, such as files or messages, are not treated as selected text.

Selections are limited to 20,000 characters. Accessibility capture preserves
indentation and trailing newlines. The copy fallback restores prior plain-text
clipboard content when possible; it does not preserve images, files, or rich
formatting.

## Privacy and updates

Text and images used for inference go to the Ollama address configured in
Settings. The default is your local machine; a remote address sends that content
to the configured server. Research sends its generated search query to You.com; page reads contact the
requested website directly. Windows OCR processes captures locally. Conversations
and captures are held in memory, while preferences are saved locally.

Snippet checks GitHub Releases for updates automatically once a day unless you
disable automatic checks. You can also check through the tray menu or
**Settings > Updates**. Update checks do not send your selected text or screenshots
to GitHub.

Download and installation require a click. In-app updates verify the download's
Tauri signature before installation. Windows installers are not Authenticode
signed, so Windows may display a SmartScreen warning. Use the official
[release page](https://github.com/zavayu/snippet/releases/latest).

## Development

Snippet uses **Rust + Tauri**, **React + TypeScript**, Windows accessibility and
OCR APIs, and Ollama for inference.

```powershell
npm ci
npm run tauri dev
```

See the [development guide](docs/development.md) for prerequisites, architecture,
configuration, and checks. The [release guide](docs/releases.md) covers signing
and publishing Windows installers.

See [benchmarking](docs/benchmarking.md) to measure search planning, web retrieval,
model loading, time to first answer text, and generation speed, or trace app requests.

Issues and contributions are welcome. Keep changes focused and include relevant
validation when changing capture, OCR, or conversation behavior.
