//! Run Snippet's production tool loop, retrieval, and streaming provider without the UI.
use snippet_lib::research;
use snippet_lib::{prompt, provider, timing};

use prompt::{PromptBuilder, PromptIntent, PromptRequest};
use provider::{ollama::OllamaProvider, GenerationEvent, GenerationRequest};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help") {
        println!("benchmark [--model NAME] [--base-url URL] [--runs N] [--output PATH] [--web] [--thinking]\nDefaults to saved model/address/thinking settings; --thinking forces thinking on.\n--web adds free You.com searches; saved API keys are never used.\nRuns are sequential, with a 180-second deadline per case. The first run is not pre-warmed.");
        return Ok(());
    }
    for arg in args.iter().filter(|arg| arg.starts_with("--")) {
        if ![
            "--model",
            "--base-url",
            "--runs",
            "--output",
            "--web",
            "--thinking",
        ]
        .contains(&arg.as_str())
        {
            return Err(format!("Unknown option: {arg}").into());
        }
    }
    let option = |name: &str| -> Result<Option<String>, String> {
        match args.iter().position(|arg| arg == name) {
            Some(index) => args
                .get(index + 1)
                .filter(|value| !value.starts_with("--"))
                .cloned()
                .map(Some)
                .ok_or_else(|| format!("Missing value for {name}")),
            None => Ok(None),
        }
    };
    let config: Value = std::env::var_os("APPDATA")
        .and_then(|path| {
            std::fs::read(PathBuf::from(path).join("com.snippet.app/settings.json")).ok()
        })
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(json!({}));
    let model = option("--model")?
        .or_else(|| config["model"].as_str().map(str::to_owned))
        .ok_or("Specify --model or choose a model in Snippet")?;
    let base = option("--base-url")?.unwrap_or_else(|| {
        config["ollamaBaseUrl"]
            .as_str()
            .unwrap_or("http://localhost:11434")
            .into()
    });
    let runs: usize = option("--runs")?.unwrap_or_else(|| "3".into()).parse()?;
    if runs == 0 {
        return Err("--runs must be positive".into());
    }
    let output =
        PathBuf::from(option("--output")?.unwrap_or_else(|| "logs/snippet-benchmark.jsonl".into()));
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let thinking =
        args.iter().any(|arg| arg == "--thinking") || config["thinking"].as_bool().unwrap_or(false);
    let web = args.iter().any(|arg| arg == "--web");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let cases = if web { vec!["direct", "auto_no_search", "auto_search", "explicit_search"] } else { vec!["direct", "auto_no_search"] };
        for run in 1..=runs {
            for case in &cases {
                eprintln!("Run {run}/{runs}: {case} ({model}, thinking={thinking})");
                let trace = timing::Trace::new(output.clone(), format!("{case}/{run}"), &model);
                trace.mark("benchmark", json!({"case": case, "run": run, "thinking": thinking, "deadline_seconds": 180, "chat_context": provider::ollama::CHAT_CONTEXT_TOKENS}));
                let provider = OllamaProvider::new(&base)?.with_trace(trace.clone());
                let cancellation = CancellationToken::new();
                let result = tokio::time::timeout(Duration::from_secs(180), run_case(case, &provider, &model, thinking, &cancellation, &trace)).await;
                let status = match result {
                    Ok(Ok(())) => "complete",
                    Ok(Err(error)) => { eprintln!("  Failed: {error}"); "error" }
                    Err(_) => { cancellation.cancel(); eprintln!("  Timed out"); "timeout" }
                };
                trace.finish(status);
            }
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    eprintln!("Traces appended to {}", output.display());
    Ok(())
}

async fn run_case(
    case: &str,
    provider: &OllamaProvider,
    model: &str,
    thinking: bool,
    cancellation: &CancellationToken,
    trace: &timing::Trace,
) -> Result<(), Box<dyn std::error::Error>> {
    let instruction = if case.ends_with("search") && case != "auto_no_search" {
        "Search the web for the official Rust release blog and summarize its latest stable release in two sentences."
    } else {
        "Explain why the sky appears blue in two sentences."
    };
    let intent = if case == "explicit_search" {
        PromptIntent::Research {
            instruction: instruction.into(),
        }
    } else {
        PromptIntent::Custom {
            instruction: instruction.into(),
        }
    };
    let preparation = trace.span("prompt_preparation");
    let mut messages = PromptBuilder::build(PromptRequest {
        selected_text: None,
        has_image: false,
        intent,
        context: None,
    })
    .map_err(|error| error.user_message())?;
    prompt::set_web_search_context(&mut messages, case != "direct");
    preparation.finish();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = GenerationRequest {
        model: model.into(),
        messages,
        thinking,
    };
    let generation = snippet_lib::agent::run(
        provider,
        request,
        case != "direct",
        case == "explicit_search",
        Vec::new(),
        cancellation.clone(),
        sender,
        |action| async move {
            match action {
                snippet_lib::agent::ToolRequest::Search(query) => {
                    research::search_traced(&query, None, cancellation, trace)
                        .await
                        .map(snippet_lib::agent::ToolOutput::Search)
                }
                snippet_lib::agent::ToolRequest::ReadWebpage(url) => {
                    snippet_lib::webpage::read(&url, cancellation)
                        .await
                        .map(snippet_lib::agent::ToolOutput::Webpage)
                }
            }
        },
        |_, _, _| {},
    );
    let consume = async {
        let mut bytes = 0;
        while let Some(event) = receiver.recv().await {
            match event {
                GenerationEvent::Delta(delta) => bytes += delta.len(),
                GenerationEvent::Reset => bytes = 0,
            }
        }
        trace.mark("answer_consumed", json!({"output_bytes": bytes}));
    };
    let (result, ()) = tokio::join!(generation, consume);
    result?;
    Ok(())
}
