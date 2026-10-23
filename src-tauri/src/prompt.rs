//! Provider-neutral prompt construction.
//!
//! This module decides what Snippet asks an LLM to do. Providers receive the
//! resulting chat messages without needing to know whether the task came from
//! a quick action or a custom instruction.

use serde::{Deserialize, Serialize};

pub mod inspector;
pub mod templates;

/// One clock snapshot, expressed in UTC and the device's local offset.
pub fn current_clock_context() -> String {
    let now = chrono::Utc::now();
    let utc_time = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let local_time = now
        .with_timezone(&chrono::Local)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    templates::render(
        templates::CLOCK,
        &[("utc_time", &utc_time), ("local_time", &local_time)],
    )
    .expect("clock template bindings")
}

/// Refresh on every turn so changing the setting also updates existing chats.
pub fn set_web_search_context(messages: &mut Vec<ChatMessage>, enabled: bool) {
    messages.retain(|message| {
        !(message.role == ChatRole::System
            && message.content.starts_with(templates::runtime_marker()))
    });
    let state = templates::text(if enabled {
        templates::SEARCH_ENABLED
    } else {
        templates::SEARCH_DISABLED
    });
    let clock = current_clock_context();
    let content = templates::render(
        templates::RUNTIME_CONTEXT,
        &[("capabilities", &state), ("clock_context", &clock)],
    )
    .expect("runtime template bindings");
    messages.insert(
        1.min(messages.len()),
        ChatMessage {
            role: ChatRole::System,
            content,
            images: None,
        },
    );
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum QuickAction {
    Explain,
    Summarize,
    Refine,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PromptIntent {
    QuickAction { action: QuickAction },
    Custom { instruction: String },
    Research { instruction: String },
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptContext {
    pub application_name: Option<String>,
    pub window_title: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    pub selected_text: Option<String>,
    pub has_image: bool,
    pub intent: PromptIntent,
    pub context: Option<PromptContext>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptBuildError {
    MissingReference,
    EmptyCustomInstruction,
}

impl PromptBuildError {
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::MissingReference => "Select text or capture a screen before asking Snippet.",
            Self::EmptyCustomInstruction => "Enter an instruction or choose a quick action.",
        }
    }
}

pub struct PromptBuilder;

impl PromptBuilder {
    pub fn build(request: PromptRequest) -> Result<Vec<ChatMessage>, PromptBuildError> {
        let selected_text = request
            .selected_text
            .as_deref()
            .filter(|text| !text.trim().is_empty());
        if selected_text.is_none()
            && !request.has_image
            && matches!(&request.intent, PromptIntent::QuickAction { .. })
        {
            return Err(PromptBuildError::MissingReference);
        }

        let instruction = instruction_for(&request.intent, request.has_image)?;
        let context = format_context(request.context);
        let text_reference = selected_text
            .map(|text| format!("\n\nReference text:\n<selected_text>\n{text}\n</selected_text>"))
            .unwrap_or_default();
        let image_reference = if request.has_image {
            "\n\nA screenshot is attached as reference material."
        } else {
            ""
        };
        let user_content =
            format!("Task:\n{instruction}{context}{text_reference}{image_reference}");

        Ok(vec![
            ChatMessage {
                role: ChatRole::System,
                content: templates::text(templates::ASSISTANT).into_owned(),
                images: None,
            },
            ChatMessage {
                role: ChatRole::User,
                content: user_content,
                images: None,
            },
        ])
    }
}

fn instruction_for(intent: &PromptIntent, has_image: bool) -> Result<String, PromptBuildError> {
    match intent {
        PromptIntent::QuickAction { action } => Ok(match action {
            QuickAction::Explain => {
                if has_image {
                    templates::text(templates::EXPLAIN_IMAGE)
                } else {
                    templates::text(templates::EXPLAIN_TEXT)
                }
            }
            QuickAction::Summarize => templates::text(templates::SUMMARIZE),
            QuickAction::Refine => templates::text(templates::REFINE),
        }
        .into_owned()),
        PromptIntent::Research { instruction } => Ok(templates::render(
            templates::RESEARCH,
            &[("instruction", instruction.trim())],
        )
        .expect("research template bindings")),
        PromptIntent::Custom { instruction } => {
            let trimmed = instruction.trim();
            if trimmed.is_empty() {
                return Err(PromptBuildError::EmptyCustomInstruction);
            }
            Ok(trimmed.into())
        }
    }
}

fn format_context(context: Option<PromptContext>) -> String {
    let Some(context) = context else {
        return String::new();
    };

    let mut lines = Vec::new();
    if let Some(application_name) = context
        .application_name
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("Application: {}", application_name.trim()));
    }
    if let Some(window_title) = context
        .window_title
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("Window title: {}", window_title.trim()));
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nAdditional context (may be incomplete):\n{}",
            lines.join("\n")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChatRole, PromptBuildError, PromptBuilder, PromptContext, PromptIntent, PromptRequest,
        QuickAction,
    };

    fn request(intent: PromptIntent) -> PromptRequest {
        PromptRequest {
            selected_text: Some("  The scheduler may preempt a process.  ".into()),
            has_image: false,
            intent,
            context: None,
        }
    }

    #[test]
    fn utc_and_local_clock_context_describe_the_same_instant() {
        let context = super::current_clock_context();
        let mut lines = context.lines();
        let utc = chrono::DateTime::parse_from_rfc3339(
            lines
                .next()
                .unwrap()
                .strip_prefix("Current UTC time: ")
                .unwrap(),
        )
        .unwrap();
        let local = chrono::DateTime::parse_from_rfc3339(
            lines
                .next()
                .unwrap()
                .strip_prefix("Device local time: ")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(utc.timestamp(), local.timestamp());
        assert!(context.contains("applying a location's time zone when known"));
    }

    #[test]
    fn runtime_capabilities_refresh_without_changing_user_or_grounding_context() {
        let mut messages = PromptBuilder::build(request(PromptIntent::Custom {
            instruction: "Can you search the web?".into(),
        }))
        .unwrap();
        messages[0].content.push_str(" Existing citation rules.");
        super::set_web_search_context(&mut messages, true);
        assert!(messages[1].content.contains("Web search is enabled"));
        assert!(messages[1].content.contains("answer yes"));
        super::set_web_search_context(&mut messages, false);
        assert_eq!(messages.len(), 3);
        assert!(messages[1].content.contains("Web search is disabled"));
        assert!(!messages[1].content.contains("Web search is enabled"));
        assert!(messages[0].content.ends_with("Existing citation rules."));
        assert!(messages[2].content.contains("Can you search the web?"));
        super::set_web_search_context(&mut messages, true);
        assert_eq!(messages.len(), 3);
        assert!(messages[1].content.contains("Web search is enabled"));
    }

    #[test]
    fn research_accepts_a_typed_question_and_keeps_the_task_separate_from_reference() {
        let messages = PromptBuilder::build(PromptRequest {
            selected_text: None,
            has_image: false,
            intent: PromptIntent::Research {
                instruction: "Compare Windows OCR and GLM-OCR".into(),
            },
            context: None,
        })
        .unwrap();
        assert!(messages[1]
            .content
            .contains("Compare Windows OCR and GLM-OCR"));
        assert!(messages[1].content.contains("cite supporting sources"));
        assert!(messages[0].content.contains("untrusted reference material"));
    }

    #[test]
    fn builds_an_explanation_prompt_without_trimming_the_reference() {
        let messages = PromptBuilder::build(request(PromptIntent::QuickAction {
            action: QuickAction::Explain,
        }))
        .unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, ChatRole::System);
        assert!(messages[0].content.contains("untrusted reference material"));
        assert!(messages[1]
            .content
            .contains("Explain the selected text clearly."));
        assert!(messages[1].content.contains(
            "<selected_text>\n  The scheduler may preempt a process.  \n</selected_text>"
        ));
    }

    #[test]
    fn builds_each_quick_action() {
        for (action, expected_instruction) in [
            (QuickAction::Explain, "Explain the selected text clearly."),
            (
                QuickAction::Summarize,
                "Summarize the selected text concisely",
            ),
            (
                QuickAction::Refine,
                "Rewrite the selected text for clarity and polish",
            ),
        ] {
            let messages =
                PromptBuilder::build(request(PromptIntent::QuickAction { action })).unwrap();
            assert!(messages[1].content.contains(expected_instruction));
        }
    }

    #[test]
    fn uses_the_custom_instruction_instead_of_a_quick_action_template() {
        let messages = PromptBuilder::build(request(PromptIntent::Custom {
            instruction: " Explain this for a new operating-systems student. ".into(),
        }))
        .unwrap();

        assert!(messages[1]
            .content
            .contains("Task:\nExplain this for a new operating-systems student."));
        assert!(!messages[1].content.contains("Define unfamiliar terms"));
    }

    #[test]
    fn includes_available_context() {
        let mut prompt_request = request(PromptIntent::QuickAction {
            action: QuickAction::Explain,
        });
        prompt_request.context = Some(PromptContext {
            application_name: Some("Visual Studio Code".into()),
            window_title: Some("scheduler.rs".into()),
        });

        let messages = PromptBuilder::build(prompt_request).unwrap();

        assert!(messages[1]
            .content
            .contains("Application: Visual Studio Code"));
        assert!(messages[1].content.contains("Window title: scheduler.rs"));
    }

    #[test]
    fn rejects_missing_selection_and_empty_custom_instructions() {
        let empty_selection = PromptBuilder::build(PromptRequest {
            selected_text: Some(" \n ".into()),
            has_image: false,
            intent: PromptIntent::QuickAction {
                action: QuickAction::Explain,
            },
            context: None,
        });
        assert_eq!(empty_selection, Err(PromptBuildError::MissingReference));

        let empty_instruction = PromptBuilder::build(request(PromptIntent::Custom {
            instruction: "  ".into(),
        }));
        assert_eq!(
            empty_instruction,
            Err(PromptBuildError::EmptyCustomInstruction)
        );
    }

    #[test]
    fn builds_a_custom_prompt_without_reference_material() {
        let messages = PromptBuilder::build(PromptRequest {
            selected_text: None,
            has_image: false,
            intent: PromptIntent::Custom {
                instruction: "What are useful ways to learn Rust?".into(),
            },
            context: None,
        })
        .unwrap();

        assert!(messages[1]
            .content
            .contains("Task:\nWhat are useful ways to learn Rust?"));
        assert!(!messages[1].content.contains("Reference text:"));
    }

    #[test]
    fn builds_a_prompt_for_an_attached_screenshot() {
        let messages = PromptBuilder::build(PromptRequest {
            selected_text: None,
            has_image: true,
            intent: PromptIntent::QuickAction {
                action: QuickAction::Explain,
            },
            context: None,
        })
        .unwrap();

        assert!(messages[1].content.contains("A screenshot is attached"));
        assert!(messages[0].content.contains("attached image"));
        assert!(messages[1]
            .content
            .contains("Explain the attached screenshot clearly."));
    }
}
