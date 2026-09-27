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

#[derive(Clone)]
pub struct OllamaProvider {
    base_url: String,
    client: Client,
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

        Ok(Self { base_url, client })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/api/{path}", self.base_url)
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
            prompt: "Extract the readable content from this screenshot. Preserve paragraphs, code, tables, and mathematical notation when possible. Return only the extracted content in Markdown; do not describe the image or add commentary.".into(),
            images: vec![image],
            stream: true,
            options: OllamaGenerateOptions {
                temperature: 0.0,
                num_predict: options.num_predict,
                num_ctx: options.num_ctx,
            },
        };
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

    async fn stream_generation(
        &self,
        request: GenerationRequest,
        cancellation: CancellationToken,
        events: UnboundedSender<GenerationEvent>,
    ) -> Result<(), ProviderError> {
        let request_body = OllamaChatRequest {
            model: request.model,
            messages: request.messages,
            stream: true,
            think: request.thinking,
        };
        let response = self
            .client
            .post(self.endpoint("chat"))
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

            let chunk = serde_json::from_str::<OllamaChatChunk>(&line)
                .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
            if let Some(error) = chunk.error {
                return Err(ProviderError::RequestFailed(error));
            }
            if let Some(content) = chunk.message.and_then(|message| message.content) {
                if !content.is_empty() {
                    events
                        .send(GenerationEvent::Delta(content))
                        .map_err(|_| ProviderError::Cancelled)?;
                }
            }
            if chunk.done {
                return Ok(());
            }
        }
    }
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

    #[test]
    fn serializes_the_thinking_preference() {
        let request = OllamaChatRequest {
            model: "qwen3:8b".into(),
            messages: vec![ChatMessage {
                role: ChatRole::User,
                content: "Explain this".into(),
                images: Some(vec!["image-data".into()]),
            }],
            stream: true,
            think: false,
        };

        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["think"], false);
        assert_eq!(value["messages"][0]["images"][0], "image-data");
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
