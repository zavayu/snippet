# Changelog

## Unreleased

- Replace separate web-search planning with native Ollama tool calls and a bounded
  search loop, retaining citations and successful tool history for follow-ups.
- Check installed-model tool support; keep ordinary chat available with a notice
  for unsupported models and require tool support for explicit Research.
- Limit searches to three per turn, reject repeated or invalid tool requests,
  and support cancellation and a five-minute request deadline.

## 0.6.1

### Added

- Show “Loading model…” only when Ollama reports the selected model is absent
  from memory, before search planning or answering. Already loaded models proceed
  without a load request; unavailable status checks fall back to normal generation.
- Offline LaTeX math rendering in responses with inline `$...$` and display `$$`
  equations, horizontal scrolling for wide equations, and preserved LaTeX source
  when copying or replacing text.

### Improved

- Guide assistant responses and Ollama OCR to format mathematical notation as
  LaTeX and escape dollar signs in prices.

### Fixed

- Keep the pending reply status visible through search planning in conversation
  mode, preventing “Thinking…” from disappearing and reappearing before the answer.

## 0.6.0

### Added

- Research quick action with free You.com web search and Ollama summaries.
- Automatic search planning for normal prompts and follow-ups; explicit Research
  bypasses the decision, and disabling web search skips planning entirely.
- Search query display, collapsed source lists, clickable individual and grouped
  citations, and source URLs in copied responses; follow-ups retain research
  evidence and preserve earlier citation numbers when retrieving new sources.
- Web search settings enabled by default, with an optional API key stored in
  Windows Credential Manager and explicit handling of provider errors and limits.
- Opt-in request timing traces covering search planning, retrieval, first answer
  content, and Ollama loading, token processing, and generation metrics.
- Benchmark tools for direct answers, automatic search decisions, and explicit
  Research, plus search-routing and freshness evaluation tools.
- Development-only request inspector for the final model request payloads, with
  opt-in capture of the last ten requests in memory and image data omitted.

### Improved

- Supply current UTC and device-local timestamps and the active search setting
  to the model, refreshed each turn and after search planning and retrieval.
- Guide answers to distinguish publication and retrieval dates from event,
  observation, and forecast times, and state when current conditions cannot be
  verified from the retrieved evidence.
- Route searches from the current instruction and conversation context while
  answering stable explanations, writing tasks, and capability questions directly.
- Expand the initial prompt input for multiline text and share keyboard behavior
  with follow-ups: Enter submits, while Shift+Enter inserts a newline.
- Keep search progress in the initial response header and resize the popup when
  source lists expand or collapse.
- Extract model instructions into Markdown templates embedded at build time,
  with validated, single-pass placeholder substitution.

### Fixed

- Paste multiline replacements as plain text with Windows line endings instead
  of typing carriage-return events, preserving paragraph breaks across editors.
  Multiline replacements leave the response on the clipboard.
- Fall back to Copy for Google Docs editor proxies instead of treating their empty
  accessibility selections as proof that no text is selected.
- Allow a bounded longer copy wait for Docs and wait for readable clipboard text
  when a web editor publishes clipboard formats in stages.
- Reject clipboard text that changes during capture and avoid treating a writable
  Google Docs input proxy as proof that the document supports replacement.

## 0.5.0

### Improved

- Detect editable, read-only, and unknown selection sources through Windows accessibility.
- Offer Replace selection for completed responses only when editability and source
  identity are confirmed, and recheck the target and selected text before insertion.
- Default screen and region text extraction to local Windows OCR using original
  capture pixels, with guidance when OCR language packs are missing.
- Add a Windows/Ollama OCR engine selector in Settings and preserve Ollama model
  and generation limits when switching engines.
- Resize the settings window when changing OCR engines.
- Position the popup beneath visible selected text when UIA supplies rectangles,
  flipping above when needed and retaining the anchor through resizing.
- Handle monitor boundaries and display scaling, with cursor placement when
  selection geometry is unavailable and manual dragging overriding the anchor.
- Open the composer after a brief accessibility check and a short automatic copy
  fallback when needed, so unsupported editors capture on the first attempt.
- Distinguish confirmed empty selections from inconclusive captures, with a
  Retry capture action that preserves the draft and targets the original app.
- Keep full capture and specific failure messages for action shortcuts and retries.
- Keep passive capture misses quiet; show a neutral capture icon and reserve
  failure messages and Retry capture for explicitly requested captures.
- Match Selection Hook's accessibility lookup ordering, adding the legacy UIA
  bridge and direct IAccessible reads before Copy, with a development-only badge.
- Try Monaco accessibility reads before fallback and suppress Copy for non-text
  object selections.
- Read highlighted text on demand through Windows UI Automation before trying
  clipboard copy, using the existing selection shortcuts.
- Keep accessibility work off the UI thread, with bounded requests and a short
  timeout before falling back to copy for unsupported or slow providers.
- Preserve selection indentation and trailing whitespace, and support multiple
  selected ranges within the existing 20,000-character limit.
- Capture the source window at shortcut invocation and reject focus changes.
- Avoid sending Copy when a supporting accessibility control reports no selection.
- Retain copy fallback for inconclusive Monaco editor selections (including VS Code).

## 0.4.0

### Added

- Continue an initial answer as a conversation with a persistent follow-up input.
- Retain the original selected text, screenshot, or OCR result and completed
  replies as context for follow-up questions.
- Start a new conversation without closing the popup.
- Copy individual replies, stop generation, and retry incomplete responses.
- New Snippet branding for the popup, tray, application icons, and favicon.

### Improved

- User messages fit their content, with separators between conversation turns.
- Response actions use icons with hover descriptions. In conversation mode, the
  New conversation action sits beside Settings and Send becomes Stop while
  generating.
- The conversation input remains visible while the transcript scrolls.
- Removed duplicate loading text and the initial response heading.

### Fixed

- Settings tabs stay on one line.
- Popup positioning runs on open rather than on the first settings click.
- Normal popup opens return to chat; explicit tray settings actions retain their
  destinations.
- Resetting invalidates late generation and OCR events from older conversations.
- Quick-action labels use readable 14px text with normal font weight.

Conversation history is kept in memory and cleared on reset or reopening. Failed
and stopped turns are marked and excluded from subsequent model context. A size
limit prevents unbounded conversation history without silently removing the
original reference.
