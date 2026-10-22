//! Ephemeral conversation context. Only successful turns enter model history.

use serde::{Deserialize, Serialize};

use crate::prompt::{ChatMessage, ChatRole};

const MAX_CONTEXT_BYTES: usize = 24_000;
const MAX_TURNS: usize = 24;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnIdentity {
    pub conversation_id: u64,
    pub turn_id: u64,
}

struct PendingTurn {
    identity: TurnIdentity,
    messages: Vec<ChatMessage>,
}

#[derive(Default)]
pub struct Conversation {
    id: u64,
    messages: Vec<ChatMessage>,
    pending: Option<PendingTurn>,
}

impl Conversation {
    pub fn ensure_current(&self, id: u64) -> Result<(), String> {
        if id != self.id {
            return Err("This conversation has been replaced. Try again.".into());
        }
        Ok(())
    }

    /// Older IPC calls cannot erase a newer session.
    pub fn reset(&mut self, id: u64) -> bool {
        if id <= self.id {
            return false;
        }
        self.id = id;
        self.messages.clear();
        self.pending = None;
        true
    }

    pub fn begin(
        &mut self,
        identity: TurnIdentity,
        initial: Option<Vec<ChatMessage>>,
        follow_up: Option<String>,
    ) -> Result<Vec<ChatMessage>, String> {
        self.ensure_current(identity.conversation_id)?;
        if self.pending.is_some() {
            return Err("Wait for the current response to finish.".into());
        }
        let messages = if let Some(messages) = initial {
            if !self.messages.is_empty() {
                return Err(
                    "Start a new conversation before sending another initial prompt.".into(),
                );
            }
            messages
        } else {
            if self.messages.is_empty() {
                return Err("Finish the first response before continuing the conversation.".into());
            }
            let instruction = follow_up.unwrap_or_default();
            let instruction = instruction.trim();
            if instruction.is_empty() {
                return Err("Enter a follow-up question.".into());
            }
            let mut messages = self.messages.clone();
            messages.push(ChatMessage {
                role: ChatRole::User,
                content: instruction.into(),
                images: None,
            });
            messages
        };
        // Bound retained text without silently removing the original reference.
        // This is a size guard, not an exact model-specific token count.
        if messages
            .iter()
            .map(|message| message.content.len())
            .sum::<usize>()
            > MAX_CONTEXT_BYTES
            || messages
                .iter()
                .filter(|message| message.role == ChatRole::User)
                .count()
                > MAX_TURNS
        {
            return Err(
                "This conversation is too long. Start a new conversation to continue.".into(),
            );
        }
        self.pending = Some(PendingTurn {
            identity,
            messages: messages.clone(),
        });
        Ok(messages)
    }

    pub fn finish(&mut self, identity: TurnIdentity, response: Option<String>) -> bool {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.identity != identity)
        {
            return false;
        }
        let mut pending = self.pending.take().expect("pending turn checked above");
        if let Some(response) = response.filter(|response| !response.trim().is_empty()) {
            pending.messages.push(ChatMessage {
                role: ChatRole::Assistant,
                content: response,
                images: None,
            });
            self.messages = pending.messages;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{PromptBuilder, PromptIntent, PromptRequest};

    fn identity(conversation_id: u64, turn_id: u64) -> TurnIdentity {
        TurnIdentity {
            conversation_id,
            turn_id,
        }
    }

    fn initial() -> Vec<ChatMessage> {
        let mut messages = PromptBuilder::build(PromptRequest {
            selected_text: Some("Original OCR text".into()),
            has_image: true,
            intent: PromptIntent::Custom {
                instruction: "Explain this".into(),
            },
            context: None,
        })
        .unwrap();
        messages[1].images = Some(vec!["original-image".into()]);
        messages
    }

    #[test]
    fn follow_ups_keep_system_reference_image_and_completed_turns() {
        let mut conversation = Conversation::default();
        conversation.reset(1);
        conversation
            .begin(identity(1, 1), Some(initial()), None)
            .unwrap();
        conversation.finish(identity(1, 1), Some("First answer".into()));
        let messages = conversation
            .begin(identity(1, 2), None, Some("Why?".into()))
            .unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, ChatRole::System);
        assert!(messages[1].content.contains("Original OCR text"));
        assert_eq!(messages[1].images.as_ref().unwrap()[0], "original-image");
        assert_eq!(messages[2].role, ChatRole::Assistant);
        assert_eq!(messages[2].content, "First answer");
        assert_eq!(messages[3].content, "Why?");
    }

    #[test]
    fn reset_rejects_late_results_and_older_reset_calls() {
        let mut conversation = Conversation::default();
        conversation.reset(1);
        conversation
            .begin(identity(1, 1), Some(initial()), None)
            .unwrap();
        assert!(conversation.reset(2));
        assert!(!conversation.reset(1));
        assert!(conversation.ensure_current(1).is_err());
        assert!(conversation.ensure_current(2).is_ok());
        assert!(!conversation.finish(identity(1, 1), Some("Late answer".into())));
        assert!(conversation
            .begin(identity(1, 2), Some(initial()), None)
            .is_err());
        assert!(conversation
            .begin(identity(2, 1), None, Some("Why?".into()))
            .is_err());
        assert!(conversation
            .begin(identity(2, 1), Some(initial()), None)
            .is_ok());
    }

    #[test]
    fn failed_or_stopped_turns_do_not_poison_history_and_can_be_retried() {
        let mut conversation = Conversation::default();
        conversation.reset(1);
        conversation
            .begin(identity(1, 1), Some(initial()), None)
            .unwrap();
        conversation.finish(identity(1, 1), Some("Answer".into()));
        conversation
            .begin(identity(1, 2), None, Some("Failed question".into()))
            .unwrap();
        conversation.finish(identity(1, 2), None);
        let messages = conversation
            .begin(identity(1, 3), None, Some("Retry question".into()))
            .unwrap();
        assert_eq!(messages.len(), 4);
        assert!(!messages
            .iter()
            .any(|message| message.content == "Failed question"));
    }

    #[test]
    fn rejects_empty_parallel_and_oversized_follow_ups_without_losing_history() {
        let mut conversation = Conversation::default();
        conversation.reset(1);
        conversation
            .begin(identity(1, 1), Some(initial()), None)
            .unwrap();
        assert!(conversation
            .begin(identity(1, 2), Some(initial()), None)
            .is_err());
        conversation.finish(identity(1, 1), Some("Answer".into()));
        assert!(conversation
            .begin(identity(1, 2), None, Some(" ".into()))
            .is_err());
        assert!(conversation
            .begin(identity(1, 2), None, Some("x".repeat(MAX_CONTEXT_BYTES)))
            .is_err());
        assert!(conversation
            .begin(identity(1, 3), None, Some("Valid question".into()))
            .is_ok());
    }

    #[test]
    fn an_empty_or_failed_first_reply_can_be_retried_but_not_continued() {
        let mut conversation = Conversation::default();
        conversation.reset(1);
        conversation
            .begin(identity(1, 1), Some(initial()), None)
            .unwrap();
        assert!(conversation.finish(identity(1, 1), Some(" ".into())));
        assert!(conversation
            .begin(identity(1, 2), None, Some("Why?".into()))
            .is_err());
        assert!(conversation
            .begin(identity(1, 3), Some(initial()), None)
            .is_ok());
    }

    #[test]
    fn long_conversations_stop_at_the_limit_without_dropping_the_reference() {
        let mut conversation = Conversation::default();
        conversation.reset(1);
        conversation
            .begin(identity(1, 1), Some(initial()), None)
            .unwrap();
        conversation.finish(identity(1, 1), Some("Answer".into()));
        for turn in 2..=MAX_TURNS as u64 {
            let messages = conversation
                .begin(identity(1, turn), None, Some("Why?".into()))
                .unwrap();
            assert_eq!(
                messages
                    .iter()
                    .filter(|message| message.role == ChatRole::System)
                    .count(),
                1
            );
            assert!(messages[1].content.contains("Original OCR text"));
            conversation.finish(identity(1, turn), Some("Answer".into()));
        }
        assert!(conversation
            .begin(identity(1, MAX_TURNS as u64 + 1), None, Some("Why?".into()))
            .is_err());
        assert_eq!(
            conversation.messages[1].images.as_ref().unwrap()[0],
            "original-image"
        );
    }
}
