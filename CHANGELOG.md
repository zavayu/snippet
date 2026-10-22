# Changelog

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
