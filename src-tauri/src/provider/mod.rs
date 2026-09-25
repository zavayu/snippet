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

#[derive(Debug, Clone)]
pub enum GenerationEvent {
    Delta(String),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub name: String,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError>;

    async fn stream_generation(
        &self,
        request: GenerationRequest,
        cancellation: CancellationToken,
        events: UnboundedSender<GenerationEvent>,
    ) -> Result<(), ProviderError>;
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
