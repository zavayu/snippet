# Benchmarking Snippet

The benchmark exercises the production prompt builder, native Ollama tool loop,
You.com retrieval, evidence attachment, and streamed answer generation. It runs
sequentially against your configured Ollama model and address. It does not
simulate model responses.

From the repository root in PowerShell:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --example benchmark -- --runs 3 --web --output C:/snippet/logs/baseline.jsonl
python scripts/summarize_benchmark.py logs/baseline.jsonl --output logs/baseline.md
```

Omit `--web` to benchmark only local requests. With `--web`, the two research cases
use free You.com access, never saved account credentials. A quota, transport, or
model failure is recorded as a failed request, rather than a successful latency
sample. Each case has a 180-second deadline. Files append, so choose a new output
path for a fresh experiment.

Cases:

| Case | Work |
|---|---|
| `direct` | Two-sentence stable explanation, with search disabled |
| `auto_no_search` | Same instruction, with the search tool available |
| `auto_search` | Request for current Rust release information, with model-selected search calls |
| `explicit_search` | Same research instruction, with forced Research |

The model still makes the automatic decision: inspect `web_search_start` events to verify what happened. Case names express the intended
path, not an assertion that the model followed it. Answers are consumed and their
byte counts recorded, but answer quality is not scored. The direct and automatic
local prompts share the same instruction; the automatic path exposes the search tool and its selection instructions.

## What the trace measures

One JSON object is written per request, with a monotonic timeline in milliseconds:

- `prompt_preparation`: benchmark prompt construction.
- `read_webpage`: public HTML download and extraction when a result is read.
- `web_search`: total retrieval; nested spans separate free MCP initialization,
  the initialized notification, search execution, and source parsing.
- `answer_request`, `answer_headers`, `answer_first_chunk`, and
  `answer_first_content`: milestones for answer startup. First content means
  streamed text; it excludes reasoning-only chunks and can include a tool-step
  preamble that the UI later clears.
- `answer_generation`: one streamed model step. Tool-using turns contain multiple
  steps; the last content-producing step is the final answer.
- `*.ollama`: server-reported loading, prompt processing, generation durations,
  token counts, and tokens per second. Missing server metrics remain absent.

Spans record their end timestamp and duration. `completed: false` means the stage
exited early (error, cancellation, or timeout). Parent and child spans overlap;
do not add them together. Engine metrics overlap the corresponding HTTP call.

## Trace normal app requests

Create the destination folder, set `SNIPPET_TRACE` to an absolute file path, then
launch the app from that shell:

```powershell
New-Item -ItemType Directory -Force C:/snippet/logs | Out-Null
$env:SNIPPET_TRACE = 'C:/snippet/logs/app-trace.jsonl'
npm run tauri dev
```

Exit an existing Snippet instance first: its single-instance behavior otherwise
keeps the old process, which cannot inherit the new environment variable. Only a
build containing the instrumentation writes traces. Remove the variable to turn
tracing off:

```powershell
Remove-Item Env:SNIPPET_TRACE
```

App tracing starts when the conversation generation is accepted. It covers text
and image answer generation and follow-ups, including the first content forwarded
to the frontend. It does not yet measure selection capture, OCR, prompt building
before acceptance, frontend rendering, or time to painted pixels. The standalone
benchmark excludes Tauri/UI work and conversation persistence, so it is a backend
baseline, not a full user-perceived latency benchmark.

Tracing is off by default. It records model names, timings, counts, configuration
flags, and statuses. It does not record prompts, selected text, queries, URLs,
answer text, API keys, or Ollama server addresses. Writes happen once per finished
request; a trace write failure does not fail the answer. A process crash can leave
the active request without a trace record.

## Model context

Every model step uses the same 16,384-token context capacity so tool use does not
change the runner's context size between requests. The separate planner and its
`--planning-context` override have been removed. OCR retains its own settings.

For repeatable comparisons, keep model, thinking setting, prompts, power mode,
other GPU activity, and Ollama version fixed. The first run is deliberately not
pre-warmed or forcibly unloaded: inspect `load_ms` to see its actual state. The
first sample must not automatically be labeled cold. Use more repetitions for
credible percentiles, and benchmark cold startup separately if needed. Generated
length and retrieved evidence can vary between runs; token counts and generation
rate help separate these from startup delays.

## Evaluate search decisions and answer behavior

Speed alone does not establish whether a search was appropriate. Run the separate
behavioral evaluation against the same model:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --example evaluate_search -- gemma4:e4b logs/search-evaluation.json
```

The evaluation uses Snippet's production tool loop and answer prompts with synthetic
inputs and synthetic tool results. It makes no web requests. The model must
support native tools. It checks 30 routing cases twice, including
capability questions, stable explanations, text transformations with search-related
reference text, current facts, polite lookup requests, and follow-ups. A routing
mismatch or invalid tool request makes the evaluation fail.

The report also saves generated answers for human review: the two-turn capability
conversation, web search disabled, a concise answer supported by synthetic
evidence, and a capability question accompanied by irrelevant external evidence.
These answer samples are not automatically scored for semantic correctness.
Passing this finite set is a regression check for the selected model, not a
guarantee for every wording or another model. Use a fresh output filename when
comparing model or prompt changes; the JSON report is overwritten on each run.

For freshness handling, use the synthetic stale-clock, weather, and event-date
fixtures. This also makes no web requests and saves answers for human review:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --example evaluate_freshness -- gemma4:e4b logs/freshness-evaluation.json
```

The clock cases check model answers against the injected current UTC/device-local
timestamps, with and without stale search excerpts. Weather and event cases check
how the model handles incomplete evidence; they do not add a live weather provider.
Review time-zone conversions against the captured timestamp, since the model
still performs those conversions.
