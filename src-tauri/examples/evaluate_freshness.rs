//! Synthetic freshness checks against a real model; performs no web requests.
use snippet_lib::{prompt, provider, timing};
#[path = "../src/research.rs"]
#[allow(dead_code)]
mod research;
use provider::{ollama::OllamaProvider, GenerationEvent, GenerationRequest, LlmProvider};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = args
        .first()
        .ok_or("Usage: evaluate_freshness MODEL [OUTPUT.json]")?;
    let output = PathBuf::from(
        args.get(1)
            .map(String::as_str)
            .unwrap_or("logs/freshness-evaluation.json"),
    );
    let provider = OllamaProvider::new(
        std::env::var("SNIPPET_TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:11434".into()),
    )?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let cases = [
            ("utc_clock", "What time is it in UTC?", vec![], "Should answer from the current injected UTC timestamp without web evidence."),
            ("dallas_clock", "What time is it in Dallas, TX?", vec![], "Should use the current clock context and Central time, without a search or a forced refusal."),
            ("cached_clocks", "What time is it in Dallas, TX?", vec![
                ("Dallas clock", "9:42:09 pm CDT on Wednesday, October 21, 2026", None),
                ("Dallas time now", "05:17:59 AM on Thursday, October 22, 2026", None),
                ("Current Dallas time", "09:23:37 on Tuesday, July 14, 2026", None),
            ], "Should use the injected current clock and Dallas time zone, ignoring cached clock readings."),
            ("undated_weather", "What is the weather in Dallas right now?", vec![
                ("Dallas weather", "Current weather: 75 F and sunny. This excerpt has no observation time.", Some("2026-10-22")),
            ], "Should not claim that 75 F is the current temperature based only on a page date."),
            ("historical_weather", "What temperature was reported on July 14, 2026 in this excerpt?", vec![
                ("Dallas weather archive", "Dallas recorded 95 F at noon CDT on July 14, 2026.", Some("2026-07-14")),
            ], "Should answer the historical question with 95 F and a citation; old evidence is appropriate for this task."),
            ("old_event_new_page", "What happened in Dallas today according to these results?", vec![
                ("Dallas station opens", "A new station opened in Dallas on July 14, 2026. This article was republished today.", Some("2026-10-22")),
            ], "Should not report the July 14 event as happening today just because the page was republished."),
        ];
        let mut results = vec![];
        for (name, instruction, excerpts, expected) in cases {
            eprintln!("Evaluating {name}");
            let mut messages = prompt::PromptBuilder::build(prompt::PromptRequest { selected_text: None, has_image: false, intent: prompt::PromptIntent::Custom { instruction: instruction.into() }, context: None }).unwrap();
            prompt::set_web_search_context(&mut messages, true);
            let sources: Vec<_> = excerpts.into_iter().enumerate().map(|(index, (title, excerpt, date))| research::Source { id: index + 1, title: title.into(), url: format!("https://example.com/{name}/{index}"), excerpt: excerpt.into(), published_at: date.map(str::to_owned) }).collect();
            if !sources.is_empty() { research::ground(&mut messages, &sources); }
            let clock_context = prompt::current_clock_context();
            let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let generate = provider.stream_generation(GenerationRequest { model: model.into(), messages, thinking: false }, CancellationToken::new(), sender);
            let collect = async { let mut text = String::new(); while let Some(GenerationEvent::Delta(delta)) = receiver.recv().await { text.push_str(&delta); } text };
            let (status, response) = tokio::time::timeout(Duration::from_secs(90), async { tokio::join!(generate, collect) }).await?;
            status?;
            results.push(json!({"case":name,"instruction":instruction,"expected":expected,"clock_context":clock_context,"response":response}));
        }
        if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) { std::fs::create_dir_all(parent)?; }
        std::fs::write(&output, serde_json::to_string_pretty(&json!({"model":model,"reference_utc_date":chrono::Utc::now().format("%Y-%m-%d").to_string(),"note":"Synthetic excerpts, no web requests; semantic results require human review.","results":results}))?)?;
        eprintln!("Saved {}", output.display());
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
