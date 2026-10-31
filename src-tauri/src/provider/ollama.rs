//! Ollama's local HTTP and newline-delimited streaming API.

use std::{io, time::Duration};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::mpsc::UnboundedSender,
};
use tokio_util::{io::StreamReader, sync::CancellationToken};

use super::{
    GenerationEvent, GenerationRequest, LlmProvider, ModelInfo, OcrGenerationOptions, ProviderError,
};
use crate::prompt::ChatMessage;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
// Bound malformed responses independently of the agent's per-tool budgets.
const MAX_TOOL_CALLS_PER_RESPONSE: usize = 16;

fn model_name(name: &str) -> String {
    if name.rsplit('/').next().unwrap_or(name).contains(':') {
        name.to_owned()
    } else {
        format!("{name}:latest")
    }
}
// Keep planning and answering on the same loaded Ollama runner. Changing num_ctx
// between requests can force a reload even when the model is unchanged.
pub const CHAT_CONTEXT_TOKENS: u32 = 16_384;

#[derive(Clone)]
pub struct OllamaProvider {
    base_url: String,
    client: Client,
    trace: crate::timing::Trace,
}

impl OllamaProvider {
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, ProviderError> {
        let base_url = base_url.as_ref().trim().trim_end_matches('/').to_owned();
        let url = reqwest::Url::parse(&base_url).map_err(|error| {
            ProviderError::RequestFailed(format!("Invalid Ollama address: {error}"))
        })?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(ProviderError::RequestFailed(
                "Invalid Ollama address.".into(),
            ));
        }

        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|error| ProviderError::RequestFailed(error.to_string()))?;

        Ok(Self {
            base_url,
            client,
            trace: crate::timing::Trace::default(),
        })
    }

    pub fn with_trace(mut self, trace: crate::timing::Trace) -> Self {
        self.trace = trace;
        self
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/api/{path}", self.base_url)
    }

    /// Check once per turn, before routing or answering. Status lookup is best
    /// effort; older servers must still be able to answer normally.
    pub async fn prepare_model(
        &self,
        model: &str,
        cancellation: &CancellationToken,
        loading: impl Fn(bool) + Send,
    ) -> Result<(), ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let check = async {
            let response = self
                .client
                .get(self.endpoint("ps"))
                .timeout(Duration::from_secs(1))
                .send()
                .await
                .ok()?;
            if !response.status().is_success() {
                return None;
            }
            response.json::<OllamaTagsResponse>().await.ok()
        };
        let running = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            result = check => result,
        };
        let Some(running) = running else {
            return Ok(());
        };
        if running
            .models
            .iter()
            .any(|entry| model_name(&entry.name) == model_name(model))
        {
            return Ok(());
        }

        loading(true);
        let load = async {
            // Use the answer context size so planning does not immediately
            // replace the runner we just loaded.
            let body = serde_json::json!({
                "model": model, "messages": [], "stream": false,
                "options": {"num_ctx": CHAT_CONTEXT_TOKENS}
            });
            let response = self
                .client
                .post(self.endpoint("chat"))
                .timeout(Duration::from_secs(300))
                .json(&body)
                .send()
                .await
                .map_err(map_connection_error)?;
            let value: serde_json::Value = ensure_success(response)
                .await?
                .json()
                .await
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
                return Err(ProviderError::RequestFailed(error.into()));
            }
            if value.get("done").and_then(serde_json::Value::as_bool) != Some(true) {
                return Err(ProviderError::InvalidResponse(
                    "Incomplete model load response".into(),
                ));
            }
            Ok(())
        };
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(ProviderError::Cancelled),
            result = load => result,
        };
        loading(false);
        result
    }

    pub async fn stream_ocr(
        &self,
        model: String,
        image: String,
        options: OcrGenerationOptions,
        cancellation: CancellationToken,
        events: UnboundedSender<GenerationEvent>,
    ) -> Result<(), ProviderError> {
        let request_body = OllamaGenerateRequest {
            model,
            prompt: crate::prompt::templates::text(crate::prompt::templates::OCR).into_owned(),
            images: vec![image],
            stream: true,
            options: OllamaGenerateOptions {
                temperature: 0.0,
                num_predict: options.num_predict,
                num_ctx: options.num_ctx,
            },
        };
        crate::prompt::inspector::record("ocr", &request_body);
        let response = self
            .client
            .post(self.endpoint("generate"))
            .json(&request_body)
            .send()
            .await
            .map_err(map_connection_error)?;
        let response = ensure_success(response).await?;

        let byte_stream = response
            .bytes_stream()
            .map(|result| result.map_err(io::Error::other));
        let reader = StreamReader::new(byte_stream);
        let mut lines = BufReader::new(reader).lines();

        let mut output = OcrOutputGuard::default();
        loop {
            let line = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                line = lines.next_line() => line.map_err(|error| ProviderError::InvalidResponse(error.to_string()))?,
            };

            let Some(line) = line else {
                return Err(ProviderError::InvalidResponse(
                    "Ollama ended the OCR response before marking it complete.".into(),
                ));
            };
            if line.trim().is_empty() {
                continue;
            }

            let chunk = serde_json::from_str::<OllamaGenerateChunk>(&line)
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            if let Some(error) = chunk.error {
                if let Some(content) = output.finish() {
                    events
                        .send(GenerationEvent::Delta(content))
                        .map_err(|_| ProviderError::Cancelled)?;
                    return Ok(());
                }
                return Err(ProviderError::RequestFailed(error));
            }
            if let Some(content) = chunk.response {
                if !content.is_empty() {
                    if let Some(extracted_text) = output.push(&content) {
                        events
                            .send(GenerationEvent::Delta(deduplicate_ocr_output(
                                &extracted_text,
                            )))
                            .map_err(|_| ProviderError::Cancelled)?;
                        return Ok(());
                    }
                }
            }
            if chunk.done {
                if let Some(content) = output.finish() {
                    events
                        .send(GenerationEvent::Delta(content))
                        .map_err(|_| ProviderError::Cancelled)?;
                }
                return Ok(());
            }
        }
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let response = self
            .client
            .get(self.endpoint("tags"))
            .send()
            .await
            .map_err(map_connection_error)?;
        let response = ensure_success(response).await?;
        let payload = response
            .json::<OllamaTagsResponse>()
            .await
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;

        Ok(payload
            .models
            .into_iter()
            .map(|model| ModelInfo { name: model.name })
            .collect())
    }

    /// Inspect the exact installed model; missing metadata means unknown support.
    async fn supports_tools(
        &self,
        model: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<bool>, ProviderError> {
        let task = async {
            let response = self
                .client
                .post(self.endpoint("show"))
                .timeout(Duration::from_secs(10))
                .json(&serde_json::json!({"model": model}))
                .send()
                .await
                .map_err(map_connection_error)?;
            let value: serde_json::Value = ensure_success(response)
                .await?
                .json()
                .await
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            Ok(tool_support(&value))
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(ProviderError::Cancelled),
            result = task => result,
        }
    }
    async fn stream_chat(
        &self,
        request: GenerationRequest,
        cancellation: CancellationToken,
        events: UnboundedSender<GenerationEvent>,
        tools: Vec<serde_json::Value>,
    ) -> Result<ChatMessage, ProviderError> {
        let mut answer = ChatMessage {
            role: crate::prompt::ChatRole::Assistant,
            content: String::new(),
            images: None,
            tool_calls: Vec::new(),
            tool_name: None,
            thinking: None,
        };
        let span = self.trace.span("answer_generation");
        self.trace.mark(
            "answer_request",
            serde_json::json!({"thinking": request.thinking, "num_ctx": CHAT_CONTEXT_TOKENS}),
        );
        let mut first_content = true;
        let mut first_chunk = true;
        let request_body = OllamaChatRequest {
            model: request.model,
            messages: request.messages,
            stream: true,
            tools,
            think: request.thinking,
            options: OllamaChatOptions {
                num_ctx: CHAT_CONTEXT_TOKENS,
            },
        };
        crate::prompt::inspector::record("answer_generation", &request_body);
        let send = async {
            let response = self
                .client
                .post(self.endpoint("chat"))
                .timeout(Duration::from_secs(180))
                .json(&request_body)
                .send()
                .await
                .map_err(map_connection_error)?;
            ensure_success(response).await
        };
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            result = send => result?,
        };
        self.trace.mark("answer_headers", serde_json::json!({}));

        let byte_stream = response
            .bytes_stream()
            .map(|result| result.map_err(io::Error::other));
        let reader = StreamReader::new(byte_stream);
        let mut lines = BufReader::new(reader).lines();

        loop {
            let line = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                line = lines.next_line() => line.map_err(|error| ProviderError::InvalidResponse(error.to_string()))?,
            };

            let Some(line) = line else {
                return Err(ProviderError::InvalidResponse(
                    "Ollama ended the response before marking it complete.".into(),
                ));
            };
            if line.trim().is_empty() {
                continue;
            }
            if line.len() > 128_000 {
                return Err(ProviderError::InvalidResponse(
                    "Ollama returned an oversized response chunk.".into(),
                ));
            }

            if first_chunk {
                self.trace.mark("answer_first_chunk", serde_json::json!({}));
                first_chunk = false;
            }
            let chunk = serde_json::from_str::<OllamaChatChunk>(&line)
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            if let Some(error) = chunk.error {
                return Err(ProviderError::RequestFailed(error));
            }
            if let Some(message) = &chunk.message {
                if let Some(thinking) = &message.thinking {
                    let accumulated = answer.thinking.get_or_insert_with(String::new);
                    accumulated.push_str(thinking);
                    if accumulated.len() > 64_000 {
                        return Err(ProviderError::RequestFailed(
                            "The model's reasoning exceeded the response limit.".into(),
                        ));
                    }
                }
                answer.tool_calls.extend(message.tool_calls.clone());
                if answer.tool_calls.len() > MAX_TOOL_CALLS_PER_RESPONSE {
                    return Err(ProviderError::RequestFailed(
                        "The model requested too many tools in one step.".into(),
                    ));
                }
            }
            if let Some(content) = chunk.message.and_then(|message| message.content) {
                if !content.is_empty() {
                    if first_content {
                        self.trace
                            .mark("answer_first_content", serde_json::json!({}));
                        first_content = false;
                    }
                    answer.content.push_str(&content);
                    if answer.content.len() > 64_000 {
                        return Err(ProviderError::RequestFailed(
                            "The model response is too large.".into(),
                        ));
                    }
                    events
                        .send(GenerationEvent::Delta(content))
                        .map_err(|_| ProviderError::Cancelled)?;
                }
            }
            if chunk.done {
                if self.trace.enabled() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                        self.trace.ollama("answer", &value);
                    }
                }
                span.finish();
                return Ok(answer);
            }
        }
    }
}

fn tool_support(value: &serde_json::Value) -> Option<bool> {
    value
        .get("capabilities")
        .and_then(serde_json::Value::as_array)
        .map(|caps| caps.iter().any(|cap| cap.as_str() == Some("tools")))
}

fn map_connection_error(error: reqwest::Error) -> ProviderError {
    if error.is_connect() || error.is_timeout() {
        ProviderError::Unavailable(error.to_string())
    } else {
        ProviderError::RequestFailed(error.to_string())
    }
}

async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    if response.status().is_success() {
        return Ok(response);
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let user_message = if status == StatusCode::NOT_FOUND && body.to_lowercase().contains("model") {
        "The selected Ollama model is not installed.".into()
    } else {
        format!("Ollama returned {status}.")
    };
    Err(ProviderError::RequestFailed(user_message))
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    name: String,
}

#[derive(Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    think: bool,
    options: OllamaChatOptions,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct OllamaChatOptions {
    num_ctx: u32,
}

#[derive(Serialize)]
struct OllamaGenerateRequest {
    model: String,
    prompt: String,
    images: Vec<String>,
    stream: bool,
    options: OllamaGenerateOptions,
}

#[derive(Serialize)]
struct OllamaGenerateOptions {
    temperature: f32,
    num_predict: u32,
    num_ctx: u32,
}

#[derive(Default)]
struct OcrOutputGuard {
    output: String,
}

impl OcrOutputGuard {
    fn push(&mut self, delta: &str) -> Option<String> {
        self.output.push_str(delta);
        repeated_block_start(&self.output).map(|start| self.output[..start].to_owned())
    }

    fn finish(self) -> Option<String> {
        (!self.output.trim().is_empty()).then(|| deduplicate_ocr_output(&self.output))
    }
}

fn repeated_block_start(text: &str) -> Option<usize> {
    const MIN_BLOCK_BYTES: usize = 64;
    const MAX_BLOCK_BYTES: usize = 2_048;
    const REPEAT_COUNT: usize = 3;

    let bytes = text.as_bytes();
    let max_block = (bytes.len() / REPEAT_COUNT).min(MAX_BLOCK_BYTES);
    if max_block < MIN_BLOCK_BYTES {
        return None;
    }

    for block_len in (MIN_BLOCK_BYTES..=max_block).rev() {
        let start = bytes.len() - block_len * REPEAT_COUNT;
        if !text.is_char_boundary(start) {
            continue;
        }
        let first = &bytes[start..start + block_len];
        let second = &bytes[start + block_len..start + block_len * 2];
        let third = &bytes[start + block_len * 2..];
        if first == second && second == third {
            return Some(start);
        }
    }

    None
}

fn deduplicate_ocr_output(text: &str) -> String {
    const MIN_MATCHED_LINES: usize = 4;
    const MIN_MATCHED_CHARS: usize = 180;

    let lines = text.lines().collect::<Vec<_>>();
    let normalized = lines
        .iter()
        .map(|line| normalize_ocr_line(line))
        .collect::<Vec<_>>();
    let mut remove = vec![false; lines.len()];

    for duplicate_start in 0..lines.len() {
        if remove[duplicate_start]
            || normalized[duplicate_start].is_empty()
            || is_structured_ocr_line(lines[duplicate_start])
        {
            continue;
        }

        for original_start in 0..duplicate_start {
            if remove[original_start]
                || !similar_ocr_lines(&normalized[original_start], &normalized[duplicate_start])
            {
                continue;
            }

            let passage = matching_passage(&lines, &normalized, original_start, duplicate_start);
            if passage.matched_lines >= MIN_MATCHED_LINES
                && passage.matched_chars >= MIN_MATCHED_CHARS
            {
                for line in remove
                    .iter_mut()
                    .take(passage.duplicate_end + 1)
                    .skip(duplicate_start)
                {
                    *line = true;
                }
                break;
            }
        }
    }

    let retained = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (!remove[index]).then_some(*line))
        .collect::<Vec<_>>();

    remove_empty_code_fences(retained)
        .join("\n")
        .trim()
        .to_owned()
}

fn remove_empty_code_fences(lines: Vec<&str>) -> Vec<&str> {
    let mut retained = Vec::with_capacity(lines.len());
    let mut index = 0;

    while index < lines.len() {
        if lines[index].trim_start().starts_with("```") {
            if let Some(closing_offset) = lines[index + 1..]
                .iter()
                .position(|line| line.trim_start().starts_with("```"))
            {
                let closing_index = index + closing_offset + 1;
                if lines[index + 1..closing_index]
                    .iter()
                    .all(|line| line.trim().is_empty())
                {
                    index = closing_index + 1;
                    continue;
                }
            }
        }

        retained.push(lines[index]);
        index += 1;
    }

    retained
}

struct MatchingPassage {
    matched_lines: usize,
    matched_chars: usize,
    duplicate_end: usize,
}

fn matching_passage(
    lines: &[&str],
    normalized: &[String],
    mut original: usize,
    mut duplicate: usize,
) -> MatchingPassage {
    let mut matched_lines = 0;
    let mut matched_chars = 0;
    let mut duplicate_end = duplicate;

    while original < lines.len() && duplicate < lines.len() {
        if normalized[original].is_empty() {
            original += 1;
            continue;
        }
        if normalized[duplicate].is_empty() {
            duplicate += 1;
            continue;
        }
        if is_structured_ocr_line(lines[original]) || is_structured_ocr_line(lines[duplicate]) {
            break;
        }
        if !similar_ocr_lines(&normalized[original], &normalized[duplicate]) {
            break;
        }

        matched_lines += 1;
        matched_chars += normalized[duplicate].len();
        duplicate_end = duplicate;
        original += 1;
        duplicate += 1;
    }

    MatchingPassage {
        matched_lines,
        matched_chars,
        duplicate_end,
    }
}

fn normalize_ocr_line(line: &str) -> String {
    let mut normalized = String::new();
    let mut previous_was_space = true;

    for character in line.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
            previous_was_space = false;
        } else if !previous_was_space {
            normalized.push(' ');
            previous_was_space = true;
        }
    }

    normalized.trim().to_owned()
}

fn similar_ocr_lines(first: &str, second: &str) -> bool {
    if first == second {
        return true;
    }
    if first.len().min(second.len()) < 24 {
        return false;
    }

    let first_words = first.split_whitespace().collect::<Vec<_>>();
    let second_words = second.split_whitespace().collect::<Vec<_>>();
    let shared_words = first_words
        .iter()
        .filter(|word| second_words.contains(word))
        .count();

    shared_words * 10 >= (first_words.len() + second_words.len()) * 4
}

fn is_structured_ocr_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('|') || trimmed.starts_with("+-") || trimmed.starts_with("┌")
}

#[derive(Deserialize)]
struct OllamaChatChunk {
    #[serde(default)]
    message: Option<OllamaAssistantMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct OllamaAssistantMessage {
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    tool_calls: Vec<crate::prompt::ToolCall>,
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct OllamaGenerateChunk {
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        deduplicate_ocr_output, repeated_block_start, OllamaChatChunk, OllamaChatRequest,
        OllamaGenerateChunk, OllamaGenerateRequest,
    };
    use crate::prompt::{ChatMessage, ChatRole};

    // Serve one response per connection and inspect the complete request, so
    // warm-model tests also prove that no extra load request is sent.
    fn preparation_server(
        replies: Vec<(&'static str, &'static str, &'static str)>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{BufRead, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (path, status, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(&mut stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with(path), "{line}");
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut payload = vec![0; length];
                reader.read_exact(&mut payload).unwrap();
                if path.starts_with("POST") {
                    let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
                    assert_eq!(value["model"], "test");
                    assert_eq!(value["messages"], serde_json::json!([]));
                    assert_eq!(value["stream"], false);
                    assert_eq!(value["options"]["num_ctx"], super::CHAT_CONTEXT_TOKENS);
                }
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        (base, server)
    }

    #[tokio::test]
    async fn prepares_only_models_confirmed_absent_and_falls_back_on_failed_checks() {
        for (status, body, cold) in [
            ("200 OK", r#"{"models":[{"name":"test:latest"}]}"#, false),
            ("200 OK", r#"{"models":[{"name":"test"}]}"#, false),
            ("200 OK", r#"{"models":[]}"#, true),
            ("200 OK", r#"{"models":[{"name":"other:latest"}]}"#, true),
            ("404 Not Found", "{}", false),
            ("200 OK", "invalid json", false),
        ] {
            let mut replies = vec![("GET /api/ps ", status, body)];
            if cold {
                replies.push((
                    "POST /api/chat ",
                    "200 OK",
                    r#"{"done":true,"done_reason":"load"}"#,
                ));
            }
            let (base, server) = preparation_server(replies);
            let provider = super::OllamaProvider::new(base).unwrap();
            let phases = std::sync::Mutex::new(Vec::new());
            provider
                .prepare_model(
                    "test",
                    &tokio_util::sync::CancellationToken::new(),
                    |loading| {
                        phases.lock().unwrap().push(loading);
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                *phases.lock().unwrap(),
                if cold { vec![true, false] } else { vec![] }
            );
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn model_load_errors_clear_the_status_and_propagate() {
        for (status, body) in [
            ("500 Internal Server Error", r#"{"error":"out of memory"}"#),
            ("200 OK", r#"{"error":"load failed"}"#),
            ("200 OK", r#"{"done":false}"#),
        ] {
            let (base, server) = preparation_server(vec![
                ("GET /api/ps ", "200 OK", r#"{"models":[]}"#),
                ("POST /api/chat ", status, body),
            ]);
            let provider = super::OllamaProvider::new(base).unwrap();
            let phases = std::sync::Mutex::new(Vec::new());
            assert!(provider
                .prepare_model(
                    "test",
                    &tokio_util::sync::CancellationToken::new(),
                    |loading| {
                        phases.lock().unwrap().push(loading);
                    }
                )
                .await
                .is_err());
            assert_eq!(*phases.lock().unwrap(), vec![true, false]);
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn preparation_can_be_cancelled_during_status_check_or_loading() {
        use std::io::{Read, Write};
        for during_load in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let (connected, connection) = tokio::sync::oneshot::channel();
            let (release, released) = std::sync::mpsc::channel();
            let server = std::thread::spawn(move || {
                if during_load {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut buffer = [0; 4096];
                    stream.read(&mut buffer).unwrap();
                    let body = r#"{"models":[]}"#;
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
                let (_stream, _) = listener.accept().unwrap();
                connected.send(()).unwrap();
                released
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
            });
            let provider = super::OllamaProvider::new(base).unwrap();
            let cancellation = tokio_util::sync::CancellationToken::new();
            let token = cancellation.clone();
            let (sender, mut phases) = tokio::sync::mpsc::unbounded_channel();
            let task = tokio::spawn(async move {
                provider
                    .prepare_model("test", &token, |loading| {
                        sender.send(loading).unwrap();
                    })
                    .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(3), connection)
                .await
                .unwrap()
                .unwrap();
            cancellation.cancel();
            let result = tokio::time::timeout(std::time::Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                result,
                Err(crate::provider::ProviderError::Cancelled)
            ));
            let mut received = Vec::new();
            while let Some(value) = phases.recv().await {
                received.push(value);
            }
            assert_eq!(
                received,
                if during_load {
                    vec![true, false]
                } else {
                    vec![]
                }
            );
            release.send(()).unwrap();
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn traces_first_answer_content_once_and_preserves_engine_metrics() {
        use crate::{
            provider::{GenerationRequest, LlmProvider},
            timing::Trace,
        };
        use serde_json::{json, Value};
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let (offset, length) = loop {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(offset) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..offset]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    break (offset + 4, length);
                }
            };
            while request.len() < offset + length {
                let mut buffer = [0; 4096];
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            let payload: Value = serde_json::from_slice(&request[offset..offset + length]).unwrap();
            assert_eq!(payload["options"]["num_ctx"], 16_384);
            let body = [
                json!({"message":{"thinking":"private reasoning"},"done":false}),
                json!({"message":{"content":"private answer"},"done":false}),
                json!({"message":{"content":" continuation"},"done":false}),
                json!({"done":true,"load_duration":1_000_000,"eval_count":2,"eval_duration":100_000_000}),
            ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let path =
            std::env::temp_dir().join(format!("snippet-stream-trace-{}.jsonl", std::process::id()));
        let trace = Trace::new(path.clone(), "stream".into(), "test");
        let provider = super::OllamaProvider::new(base)
            .unwrap()
            .with_trace(trace.clone());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        provider
            .stream_generation(
                GenerationRequest {
                    model: "test".into(),
                    messages: vec![],
                    thinking: false,
                },
                tokio_util::sync::CancellationToken::new(),
                sender,
            )
            .await
            .unwrap();
        assert!(receiver.recv().await.is_some());
        assert!(receiver.recv().await.is_some());
        assert!(receiver.recv().await.is_none());
        trace.finish("complete");
        server.join().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("private"));
        let record: Value = serde_json::from_str(&text).unwrap();
        let events = record["events"].as_array().unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e["name"] == "answer_first_content")
                .count(),
            1
        );
        let metrics = events
            .iter()
            .find(|e| e["name"] == "answer.ollama")
            .unwrap();
        assert_eq!(metrics["detail"]["tokens_per_second"], 20.0);
        assert_eq!(metrics["detail"]["load_ms"], 1.0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn serializes_the_thinking_preference() {
        let request = OllamaChatRequest {
            tools: Vec::new(),
            model: "qwen3:8b".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "Explain this".into(),
                images: Some(vec!["image-data".into()]),
                tool_calls: Vec::new(),
                tool_name: None,
                thinking: None,
            }],
            stream: true,
            think: false,
            options: super::OllamaChatOptions { num_ctx: 16_384 },
        };

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["think"], false);
        assert_eq!(value["options"]["num_ctx"], 16_384);
        assert_eq!(value["messages"][0]["images"][0], "image-data");
    }

    #[test]
    fn reads_native_tool_capability_metadata_without_guessing_from_names() {
        assert_eq!(
            super::tool_support(&serde_json::json!({"capabilities":["completion","tools"]})),
            Some(true)
        );
        assert_eq!(
            super::tool_support(&serde_json::json!({"capabilities":["completion","vision"]})),
            Some(false)
        );
        assert_eq!(
            super::tool_support(&serde_json::json!({"capabilities":[]})),
            Some(false)
        );
        assert_eq!(super::tool_support(&serde_json::json!({})), None);
        assert_eq!(
            super::tool_support(&serde_json::json!({"capabilities":"tools"})),
            None
        );
    }

    #[test]
    fn parses_native_tool_calls_and_reasoning_but_rejects_malformed_calls() {
        let chunk: OllamaChatChunk = serde_json::from_value(serde_json::json!({
            "message":{"thinking":"Need current sources","content":"","tool_calls":[{"function":{"index":0,"name":"web_search","arguments":{"query":"Rust release"}}}]},"done":false
        })).unwrap();
        let message = chunk.message.unwrap();
        assert_eq!(message.thinking.as_deref(), Some("Need current sources"));
        assert_eq!(message.tool_calls[0].function.name, "web_search");
        assert_eq!(
            message.tool_calls[0].function.arguments["query"],
            "Rust release"
        );
        let retained = ChatMessage {
            role: ChatRole::Assistant,
            content: String::new(),
            images: None,
            tool_calls: message.tool_calls,
            tool_name: None,
            thinking: message.thinking,
        };
        let serialized = serde_json::to_value(retained).unwrap();
        assert_eq!(serialized["thinking"], "Need current sources");
        assert_eq!(
            serialized["tool_calls"][0]["function"]["arguments"]["query"],
            "Rust release"
        );
        assert!(serde_json::from_value::<OllamaChatChunk>(
            serde_json::json!({"message":{"tool_calls":[{"function":{"arguments":{}}}]}})
        )
        .is_err());
    }

    #[test]
    fn parses_a_streaming_text_chunk() {
        let chunk = serde_json::from_str::<OllamaChatChunk>(
            r#"{"message":{"role":"assistant","content":"Hello"},"done":false}"#,
        )
        .unwrap();

        assert_eq!(
            chunk.message.and_then(|message| message.content).as_deref(),
            Some("Hello")
        );
        assert!(!chunk.done);
    }

    #[test]
    fn parses_a_completed_chunk() {
        let chunk = serde_json::from_str::<OllamaChatChunk>(r#"{"done":true}"#).unwrap();
        assert!(chunk.done);
    }

    #[test]
    fn serializes_an_ocr_request_with_an_image() {
        let request = OllamaGenerateRequest {
            model: "glm-ocr:latest".into(),
            prompt: "Extract text".into(),
            images: vec!["image-data".into()],
            stream: true,
            options: super::OllamaGenerateOptions {
                temperature: 0.0,
                num_predict: 2_048,
                num_ctx: 16_384,
            },
        };

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["images"][0], "image-data");
        assert_eq!(value["options"]["temperature"], 0.0);
        assert_eq!(value["options"]["num_predict"], 2_048);
        assert_eq!(value["options"]["num_ctx"], 16_384);
    }

    #[test]
    fn parses_a_streaming_ocr_chunk() {
        let chunk = serde_json::from_str::<OllamaGenerateChunk>(
            r##"{"response":"# Extracted","done":false}"##,
        )
        .unwrap();

        assert_eq!(chunk.response.as_deref(), Some("# Extracted"));
        assert!(!chunk.done);
    }

    #[test]
    fn finds_three_repeated_ocr_blocks() {
        let block = "A sufficiently long OCR line that should not be repeated in the result.\n";
        let text = format!("Heading\n{block}{block}{block}");

        assert_eq!(repeated_block_start(&text), Some("Heading\n".len()));
    }

    #[test]
    fn allows_non_repeated_ocr_text() {
        let text = "Heading\nFirst distinct table row with enough content to exceed the guard threshold.\nSecond distinct table row with enough content to exceed the guard threshold.\n";

        assert_eq!(repeated_block_start(text), None);
    }

    #[test]
    fn removes_a_markdown_wrapped_fuzzy_repeated_passage() {
        let original = "Implemented the OCR mitigations.\n\
Settings now have separate General, Read screen, and Shortcuts pages.\n\
Read screen includes sliders for output and context window limits.\n\
Those settings are persisted and sent to Ollama on each OCR request.\n\
Normal model responses continue streaming as before; only OCR is buffered.\n";
        let duplicate = "```markdown\n\
Implemented the OCR mitigations!\n\
- Settings now have separate General, Read screen, and Shortcuts pages.\n\
- Read screen includes sliders for output and context-window limits.\n\
- Those settings are persisted and sent to Ollama on every OCR request.\n\
- Normal model responses continue streaming as before: only OCR is buffered.\n\
```\n";

        let cleaned = deduplicate_ocr_output(&format!("{original}\n{duplicate}"));

        assert_eq!(
            cleaned.matches("Implemented the OCR mitigations").count(),
            1
        );
        assert!(!cleaned.contains("```markdown"));
    }

    #[test]
    fn keeps_repeated_table_style_lines() {
        let text = "| Setting | Value |\n| --- | --- |\n| num_predict | 2048 |\n| num_ctx | 16384 |\n\n| Setting | Value |\n| --- | --- |\n| num_predict | 2048 |\n| num_ctx | 16384 |";

        assert_eq!(deduplicate_ocr_output(text), text);
    }
}
