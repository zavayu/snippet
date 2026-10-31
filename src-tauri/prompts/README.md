# Instruction templates

These Markdown files contain Snippet's model-facing instructions. The catalog in
`../src/prompt/templates.rs` embeds them with `include_str!` at build time. Editing
a template requires rebuilding the Rust app; requests never read these files from
disk. This refactor preserves the existing instruction wording.

| Files | Where they enter the model request |
| --- | --- |
| `assistant.md` | First answer system message |
| `runtime-context.md`, `clock.md`, `capabilities/*.md` | Runtime system message, refreshed for each answer |
| `actions/*.md` | Quick-action or explicit Research task in the user message |
| `tool-loop.md` | Per-turn tool selection and evidence-reuse instructions |
| `web-evidence.md` | Runtime system message and tool-result evidence rules |
| `freshness.md` | Evidence message when grounding with search results |
| `retrieval-note.md` | Retrieval metadata inside evidence JSON |
| `ocr.md` | Ollama OCR generate prompt (native Windows OCR has no model prompt) |

Templates use literal `{{name}}` placeholders. Bindings are supplied by Rust:

| Template | Bindings |
| --- | --- |
| `clock` | `utc_time`, `local_time` from one device-clock snapshot |
| `runtime-context` | `capabilities`, `clock_context` |
| `actions/research` | `instruction` |

Substitution is single-pass: placeholder-looking text inside user input stays
literal. Missing or malformed placeholders fail explicitly. Keep placeholders in
sync with their Rust bindings; the catalog test checks every template. Use the
request inspector to verify the fully assembled result after editing.

Leading spaces and blank lines in appended instruction templates separate sections
and are intentional. Final file newlines are omitted, and checkout CRLF line
endings are normalized to LF; inserted reference text retains its whitespace.
The first line of `runtime-context.md` identifies that message when refreshing it.

Rust still owns message roles and order, runtime facts, JSON schemas, image
attachment, routing, evidence limits, and HTTP options. Template edits change
instructions; they do not add tools or change the application's capabilities.

Small formatting fragments are assembled directly in Rust: `../src/prompt.rs`
adds the task heading, selected-text tags, screenshot reference, and application
context; `../src/research.rs` adds the web evidence JSON heading. These wrappers
preserve the same wording and spacing without separate Markdown files.

See [the development guide](../../docs/development.md#inspecting-model-requests)
for the opt-in final-request inspector.
