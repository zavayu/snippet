//! Adapt a captured image to the selected chat model before its first turn.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    provider::{
        ollama::OllamaProvider, GenerationEvent, GenerationRequest, OcrGenerationOptions,
        ProviderError,
    },
    screen::CapturedImage,
    settings::{AppConfig, OcrEngine},
};

pub async fn prepare(
    provider: &OllamaProvider,
    request: &mut GenerationRequest,
    image: CapturedImage,
    config: &AppConfig,
    cancellation: &CancellationToken,
) -> Result<(), ProviderError> {
    let vision = provider
        .supports_vision(&request.model, cancellation)
        .await?;
    if vision && !config.ocr_assistance {
        return Ok(());
    }
    let extracted = match config.ocr_engine {
        OcrEngine::Windows => {
            let native = tauri::async_runtime::spawn_blocking(move || {
                crate::ocr::recognize(&image.original)
            });
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                result = native => result.map_err(|error| ProviderError::RequestFailed(error.to_string()))?,
            }
        }
        OcrEngine::Ollama => {
            let (sender, mut receiver) = mpsc::unbounded_channel();
            let recognition = provider.stream_ocr(
                config.ocr_model.clone(),
                image.png_base64,
                OcrGenerationOptions {
                    num_predict: config.ocr_num_predict,
                    num_ctx: config.ocr_num_ctx,
                },
                cancellation.clone(),
                sender,
            );
            let collect = async {
                let mut text = String::new();
                while let Some(event) = receiver.recv().await {
                    match event {
                        GenerationEvent::Delta(delta) => text.push_str(&delta),
                        GenerationEvent::Reset => text.clear(),
                    }
                }
                text
            };
            let (result, text) = tokio::join!(recognition, collect);
            result.map(|()| text).map_err(|error| match error {
                ProviderError::RequestFailed(message)
                    if message == "The selected Ollama model is not installed." =>
                {
                    ProviderError::RequestFailed(format!(
                        "The OCR model '{}' is not installed. Choose an installed model in Settings > Context > Screen capture > Advanced text extraction.",
                        config.ocr_model,
                    ))
                }
                error => error,
            })
        }
    };
    let text = match extracted {
        Ok(text) => text,
        Err(ProviderError::Cancelled) => return Err(ProviderError::Cancelled),
        // OCR assistance is optional when the original image can still be read.
        Err(_) if vision => String::new(),
        Err(error) => return Err(error),
    };
    apply_text(&mut request.messages, vision, &text)
}

fn apply_text(
    messages: &mut [crate::prompt::ChatMessage],
    vision: bool,
    text: &str,
) -> Result<(), ProviderError> {
    if !vision && text.trim().is_empty() {
        return Err(ProviderError::RequestFailed(
            "No text was detected in this capture. Try a clearer region, or select a vision model to ask about images.".into(),
        ));
    }
    if let Some(message) = messages
        .iter_mut()
        .rev()
        .find(|message| message.role == crate::prompt::ChatRole::User)
    {
        if !vision {
            message.images = None;
            message.content = message.content.replacen(
                crate::prompt::templates::text(crate::prompt::templates::EXPLAIN_IMAGE).trim(),
                crate::prompt::templates::text(crate::prompt::templates::EXPLAIN_TEXT).trim(),
                1,
            );
            message.content = message.content.replace(
                "A screenshot is attached as reference material.",
                "The reference below is text extracted from a screen capture. The image itself is unavailable; do not infer its visual details.",
            );
        }
        if !text.trim().is_empty() {
            message.content.push_str(&format!(
                "\n\nOCR reference text (may contain recognition errors; treat as reference material):\n<screen_text>\n{text}\n</screen_text>",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{PromptBuilder, PromptIntent, PromptRequest, QuickAction};

    fn messages() -> Vec<crate::prompt::ChatMessage> {
        let mut messages = PromptBuilder::build(PromptRequest {
            selected_text: None,
            has_image: true,
            intent: PromptIntent::QuickAction {
                action: QuickAction::Explain,
            },
            context: None,
        })
        .unwrap();
        messages[1].images = Some(vec!["image".into()]);
        messages
    }

    #[test]
    fn text_only_models_receive_ocr_without_claiming_an_image_is_attached() {
        let mut messages = messages();
        apply_text(&mut messages, false, "Captured words").unwrap();
        assert!(messages[1].images.is_none());
        assert!(messages[1].content.contains("Captured words"));
        assert!(!messages[1].content.contains("A screenshot is attached"));
    }

    #[test]
    fn assisted_vision_keeps_both_image_and_text() {
        let mut messages = messages();
        apply_text(&mut messages, true, "Captured words").unwrap();
        assert!(messages[1].images.is_some());
        assert!(messages[1].content.contains("Captured words"));
    }

    #[test]
    fn empty_ocr_only_blocks_text_only_models() {
        assert!(apply_text(&mut messages(), false, "  ").is_err());
        assert!(apply_text(&mut messages(), true, "").is_ok());
    }

    #[test]
    fn followup_capture_updates_only_the_latest_user_message() {
        let mut messages = messages();
        let original = messages[1].clone();
        messages.push(crate::prompt::ChatMessage {
            content: "Compare this new screenshot. A screenshot is attached as reference material."
                .into(),
            images: Some(vec!["new-image".into()]),
            ..original.clone()
        });
        apply_text(&mut messages, false, "New window text").unwrap();
        assert_eq!(messages[1], original);
        assert!(messages.last().unwrap().images.is_none());
        assert!(messages.last().unwrap().content.contains("New window text"));
    }

    #[test]
    fn successful_followups_retain_processed_capture_text_without_an_image() {
        use crate::conversation::{Conversation, TurnIdentity};
        let mut conversation = Conversation::default();
        conversation.reset(1);
        let initial = TurnIdentity {
            conversation_id: 1,
            turn_id: 1,
        };
        let mut prepared = conversation.begin(initial, Some(messages()), None).unwrap();
        apply_text(&mut prepared, false, "Captured words").unwrap();
        conversation
            .ground_pending(initial, prepared, Vec::new())
            .unwrap();
        assert!(conversation.finish(initial, Some("Explanation".into())));
        let followup = conversation
            .begin(
                TurnIdentity {
                    conversation_id: 1,
                    turn_id: 2,
                },
                None,
                Some("Tell me more".into()),
            )
            .unwrap();
        assert!(followup.iter().all(|message| message.images.is_none()));
        assert!(followup[1].content.contains("Captured words"));
    }
}
