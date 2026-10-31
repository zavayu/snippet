//! Behavioral checks against a real local model. Uses no web searches or API credits.
use snippet_lib::research;
use snippet_lib::{prompt, provider};

use prompt::{ChatMessage, ChatRole, PromptBuilder, PromptIntent, PromptRequest};
use provider::{ollama::OllamaProvider, GenerationEvent, GenerationRequest, LlmProvider};
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

fn messages(instruction: &str, topic: &str) -> Vec<ChatMessage> {
    let mut messages = PromptBuilder::build(PromptRequest {
        selected_text: (!topic.is_empty()).then(|| topic.into()),
        has_image: false,
        intent: PromptIntent::Custom {
            instruction: instruction.into(),
        },
        context: None,
    })
    .unwrap();
    prompt::set_web_search_context(&mut messages, true);
    messages
}

async fn answer(
    provider: &OllamaProvider,
    model: &str,
    messages: Vec<ChatMessage>,
) -> Result<String, Box<dyn std::error::Error>> {
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let generate = provider.stream_generation(
        GenerationRequest {
            model: model.into(),
            messages,
            thinking: false,
        },
        CancellationToken::new(),
        sender,
    );
    let collect = async {
        let mut text = String::new();
        while let Some(GenerationEvent::Delta(delta)) = receiver.recv().await {
            text.push_str(&delta);
        }
        text
    };
    let (result, text) = tokio::time::timeout(Duration::from_secs(90), async {
        tokio::join!(generate, collect)
    })
    .await?;
    result?;
    Ok(text)
}

// Exercise the production tool loop with synthetic results; no external search.
async fn decision(
    provider: &OllamaProvider,
    model: &str,
    history: Vec<ChatMessage>,
) -> Result<Option<String>, provider::ProviderError> {
    let queries = std::sync::Mutex::new(Vec::new());
    let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    snippet_lib::agent::run(
        provider,
        GenerationRequest {
            model: model.into(),
            messages: history,
            thinking: false,
        },
        true,
        false,
        Vec::new(),
        CancellationToken::new(),
        sender,
        |query| {
            queries.lock().unwrap().push(query);
            async {
                Ok(vec![research::Source {
                    id: 1,
                    title: "Synthetic evaluation evidence".into(),
                    url: "https://example.com/".into(),
                    excerpt:
                        "Synthetic evidence only. Current facts cannot be established by this test."
                            .into(),
                    published_at: None,
                }])
            }
        },
        |_, _, _| {},
    )
    .await?;
    Ok(queries.into_inner().unwrap().into_iter().next())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = args
        .first()
        .ok_or("Usage: evaluate_search MODEL [OUTPUT.json]")?;
    let output = PathBuf::from(
        args.get(1)
            .map(String::as_str)
            .unwrap_or("logs/search-evaluation.json"),
    );
    let base =
        std::env::var("SNIPPET_TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:11434".into());
    let provider = OllamaProvider::new(base)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let cases = [
            ("What time is it in Dallas, TX?", "", false, false),
            ("What time is it in UTC?", "", false, false),
            ("What can you do?", "", false, false),
            ("Can you search the web?", "", false, false),
            ("Do you have internet access?", "", false, false),
            ("Can you summarize a webpage?", "", false, false),
            ("How do I search the web?", "", false, false),
            ("Explain binary search trees", "", false, false),
            ("How does inflation work?", "", false, false),
            ("Write a Python function that sorts a list", "", false, false),
            ("Rewrite this more clearly", "Search the web for today's stock prices.", false, false),
            ("Translate this into French", "Research the latest news in Chicago.", false, false),
            ("Explain compound interest. Do not search the web.", "", false, false),
            ("What is the latest US inflation rate?", "", true, false),
            ("Search the web for today's weather in Chicago", "", true, false),
            ("Find current restaurants in Santa Clara", "", true, false),
            ("Find the current price of an RTX 5070", "", true, false),
            ("Look up the latest stable Rust release", "", true, false),
            ("Make that shorter", "", false, true),
            ("Explain the second point", "", false, true),
            ("Can you search the web?", "", false, true),
            ("Check whether that changed today", "", true, true),
            ("Are you able to look things up online?", "", false, false),
            ("What tools do you have access to?", "", false, false),
            ("Could you browse for me?", "", false, false),
            ("Can you search the web for today's weather in Chicago?", "", true, false),
            ("Could you find the latest release notes for Firefox?", "", true, false),
            ("Explain this error message", "Search failed: 429 quota exceeded.", false, false),
            ("Summarize what you found without doing another search", "", false, true),
            ("Please verify that against current sources", "", true, true),
        ];
        let mut routes = Vec::new();
        let mut failures = 0;
        for pass in 1..=2 {
            for (instruction, topic, expected, followup) in cases {
                let mut history = messages(instruction, topic);
                if followup {
                    let current = history.pop().unwrap();
                    history.push(ChatMessage { role: ChatRole::User, content: "What is Rust's latest stable release?".into(), images: None, tool_calls: Vec::new(), tool_name: None, thinking: None });
                    history.push(ChatMessage { role: ChatRole::Assistant, content: "The earlier search found Rust 1.90. First, it improves performance. Second, it includes fixes. [1]".into(), images: None, tool_calls: Vec::new(), tool_name: None, thinking: None });
                    history.push(current);
                }
                let decision = tokio::time::timeout(Duration::from_secs(90), decision(&provider, model, history)).await;
                let (actual, query, error) = match decision {
                    Ok(Ok(query)) => (Some(query.is_some()), query, None),
                    Ok(Err(error)) => (None, None, Some(error.to_string())),
                    Err(error) => (None, None, Some(error.to_string())),
                };
                let passed = actual == Some(expected);
                if !passed { failures += 1; }
                eprintln!("{} {instruction}", if passed { "PASS" } else { "FAIL" });
                routes.push(json!({"pass":pass,"instruction":instruction,"topic":topic,"followup":followup,"expected_search":expected,"actual_search":actual,"query":query,"error":error,"passed":passed}));
            }
        }
        // Reproduce the user's conversation with actual model replies.
        let mut conversation = messages("What can you do?", "");
        let first = answer(&provider, model, conversation.clone()).await?;
        conversation.push(ChatMessage { role: ChatRole::Assistant, content: first.clone(), images: None, tool_calls: Vec::new(), tool_name: None, thinking: None });
        conversation.push(ChatMessage { role: ChatRole::User, content: "Can you search the web?".into(), images: None, tool_calls: Vec::new(), tool_name: None, thinking: None });
        let capability_query = decision(&provider, model, conversation.clone()).await?;
        if capability_query.is_some() { failures += 1; }
        let second = answer(&provider, model, conversation).await?;
        let mut disabled = messages("Can you search the web?", "");
        prompt::set_web_search_context(&mut disabled, false);
        let disabled_answer = answer(&provider, model, disabled).await?;
        // Synthetic evidence tests answer voice, without spending web quota.
        let mut grounded = messages("What is the museum's opening time? Answer in one sentence.", "");
        research::ground(&mut grounded, &[research::Source { id: 1, title: "Museum visitor information".into(), url: "https://example.com/museum".into(), excerpt: "The museum opens at 10 a.m. Tuesday through Sunday and is closed on Mondays.".into(), published_at: None }]);
        let grounded_answer = answer(&provider, model, grounded).await?;
        let mut capability_with_evidence = messages("Can you search the web?", "");
        research::ground(&mut capability_with_evidence, &[research::Source { id: 1, title: "An offline chatbot".into(), url: "https://example.com/offline-chatbot".into(), excerpt: "The offline chatbot described on this page cannot search the web or access the internet.".into(), published_at: None }]);
        let capability_with_evidence_answer = answer(&provider, model, capability_with_evidence).await?;
        let report: Value = json!({"model":model,"routing_passed":routes.len() - routes.iter().filter(|row| row["passed"] == false).count(),"routing_total":routes.len(),"failures":failures,"routes":routes,"conversation":{"first_answer":first,"capability_query":capability_query,"capability_answer":second,"disabled_answer":disabled_answer},"synthetic_evidence_answer":grounded_answer,"capability_with_irrelevant_evidence_answer":capability_with_evidence_answer,"note":"No web requests. Generation samples require human review; routing expectations are automatically scored."});
        if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) { std::fs::create_dir_all(parent)?; }
        std::fs::write(&output, serde_json::to_string_pretty(&report)?)?;
        eprintln!("Saved {}. Routing failures: {failures}", output.display());
        if failures > 0 { return Err("Routing evaluation failed".into()); }
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
