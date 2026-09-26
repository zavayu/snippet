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

use super::{GenerationEvent, GenerationRequest, LlmProvider, ModelInfo, ProviderError};
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

#[cfg(test)]
mod tests {
    use super::{OllamaChatChunk, OllamaChatRequest};
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
}
