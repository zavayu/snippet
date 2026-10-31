//! Provider-neutral local language-model interfaces.

pub mod ollama;

use std::fmt;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::prompt::ChatMessage;

#[derive(Debug, Clone)]
pub struct GenerationRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub thinking: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct OcrGenerationOptions {
    pub num_predict: u32,
    pub num_ctx: u32,
}

#[derive(Debug, Clone)]
pub enum GenerationEvent {
    Delta(String),
    Reset,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub name: String,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError>;

    async fn supports_tools(
        &self,
        model: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<bool>, ProviderError>;

    async fn stream_chat(
        &self,
        request: GenerationRequest,
        cancellation: CancellationToken,
        events: UnboundedSender<GenerationEvent>,
        tools: Vec<serde_json::Value>,
    ) -> Result<ChatMessage, ProviderError>;

    async fn stream_generation(
        &self,
        request: GenerationRequest,
        cancellation: CancellationToken,
        events: UnboundedSender<GenerationEvent>,
    ) -> Result<(), ProviderError> {
        let answer = self
            .stream_chat(request, cancellation, events, Vec::new())
            .await?;
        if !answer.tool_calls.is_empty() {
            return Err(ProviderError::RequestFailed(
                "The model requested an unavailable tool.".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum ProviderError {
    Unavailable(String),
    RequestFailed(String),
    InvalidResponse(String),
    Cancelled,
}

impl ProviderError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Unavailable(_) => "Ollama is not available. Start Ollama, then try again.".into(),
            Self::RequestFailed(message) => message.clone(),
            Self::InvalidResponse(_) => "Ollama returned an unreadable response. Try again.".into(),
            Self::Cancelled => "Generation cancelled.".into(),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message)
            | Self::RequestFailed(message)
            | Self::InvalidResponse(message) => formatter.write_str(message),
            Self::Cancelled => formatter.write_str("Generation cancelled"),
        }
    }
}

impl std::error::Error for ProviderError {}
