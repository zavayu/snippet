# Benchmarking Snippet

The benchmark exercises the production prompt builder, Ollama search planner,
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
| `auto_no_search` | Same instruction, with automatic search planning enabled |
| `auto_search` | Request for current Rust release information, with automatic search planning |
| `explicit_search` | Same research instruction, with forced Research |

The model still makes the automatic decision: inspect `search_skipped` and
`web_search_start` events to verify what happened. Case names express the intended
path, not an assertion that the model followed it. Answers are consumed and their
byte counts recorded, but answer quality is not scored. The direct and automatic
local prompts share the same instruction; the automatic path also adds Snippet's
normal no-new-search system reminder.

## What the trace measures

One JSON object is written per request, with a monotonic timeline in milliseconds:

- `prompt_preparation`: benchmark prompt construction.
- `search_decision` or `query_planning`: complete planner call, including loading,
  prompt processing, inference, and HTTP overhead.
- `web_search`: total retrieval; nested spans separate free MCP initialization,
  the initialized notification, search execution, and source parsing.
- `attach_evidence`: attaching results to the answer context.
- `answer_request`, `answer_headers`, `answer_first_chunk`, and
  `answer_first_content`: milestones for answer startup. First content means
  actual answer text; it excludes reasoning-only chunks.
- `answer_generation`: complete streamed answer request.
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

## Investigate context-size switching

Planning and answer generation both use 16,384 tokens by default, sharing one
context-size constant. This allows Ollama to reuse a loaded runner across both
stages. Planning still receives bounded input and generates a short query or
decision; setting a 16K capacity does not pad its prompt to 16K tokens.

The original benchmark used 4,096 for planning and 16,384 for answering. On the
measured gemma4:e4b setup, switching sizes repeatedly incurred long model-loading
delays. To reproduce that older configuration for a controlled comparison:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --example benchmark -- --runs 3 --planning-context 4096 --output C:/snippet/logs/mismatched-context.jsonl
python scripts/summarize_benchmark.py logs/mismatched-context.jsonl --output logs/mismatched-context.md
```

The override applies only to that benchmark run. Compare its planner and answer
`load_ms`, request total, and time to first answer text with a default run. Larger
context capacity uses more memory; the aligned default preserves the existing
16K answer capacity. OCR has its own model and configurable context, so this
change does not align OCR with chat or prevent reloads after switching models.

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

The evaluation uses Snippet's actual planner and answer prompts with synthetic
inputs. It makes no web requests. It checks 30 routing cases twice, including
capability questions, stable explanations, text transformations with search-related
reference text, current facts, polite lookup requests, and follow-ups. A routing
mismatch or invalid decision makes the evaluation fail.

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
