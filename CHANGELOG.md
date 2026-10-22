# Changelog

## Unreleased

### Improved

- Detect editable, read-only, and unknown selection sources through Windows accessibility.
- Offer Replace selection for completed responses only when editability and source
  identity are confirmed, and recheck the target and selected text before insertion.

- Default screen and region text extraction to local Windows OCR using original
  capture pixels, with guidance when OCR language packs are missing.
- Add a Windows/Ollama OCR engine selector in Settings and preserve Ollama model
  and generation limits when switching engines.

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
