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
    sources: Vec<crate::research::Source>,
}

#[derive(Default)]
pub struct Conversation {
    id: u64,
    messages: Vec<ChatMessage>,
    pending: Option<PendingTurn>,
    sources: Vec<crate::research::Source>,
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
        self.sources.clear();
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
                tool_calls: Vec::new(),
                tool_name: None,
                thinking: None,
            });
            messages
        };
        // Bound retained text without silently removing the original reference.
        // This is a size guard, not an exact model-specific token count.
        if messages
            .iter()
            .map(ChatMessage::context_bytes)
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
            sources: self.sources.clone(),
        });
        Ok(messages)
    }

    /// Retain fetched evidence with the pending turn so follow-ups share its sources.
    pub fn ground_pending(
        &mut self,
        identity: TurnIdentity,
        messages: Vec<ChatMessage>,
        sources: Vec<crate::research::Source>,
    ) -> Result<(), String> {
        self.ensure_current(identity.conversation_id)?;
        if messages
            .iter()
            .map(ChatMessage::context_bytes)
            .sum::<usize>()
            > MAX_CONTEXT_BYTES
        {
            return Err("Research context is too large. Try a shorter selection.".into());
        }
        let pending = self
            .pending
            .as_mut()
            .filter(|pending| pending.identity == identity)
            .ok_or("This research request has been replaced.")?;
        pending.messages = messages;
        pending.sources = sources;
        Ok(())
    }

    pub fn sources(&self) -> Vec<crate::research::Source> {
        self.sources.clone()
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
                tool_calls: Vec::new(),
                tool_name: None,
                thinking: None,
            });
            self.messages = pending.messages;
            self.sources = pending.sources;
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
    fn research_evidence_is_retained_only_for_the_current_successful_turn() {
        let mut session = Conversation::default();
        session.reset(1);
        let id = identity(1, 1);
        let mut messages = session.begin(id, Some(initial()), None).unwrap();
        messages[1].content.push_str(" Retrieved source evidence.");
        assert!(session
            .ground_pending(identity(1, 2), messages.clone(), Vec::new())
            .is_err());
        let sources = vec![crate::research::Source {
            id: 1,
            title: "First".into(),
            url: "https://example.com/".into(),
            excerpt: "Evidence".into(),
            published_at: None,
        }];
        session
            .ground_pending(id, messages, sources.clone())
            .unwrap();
        assert!(session.sources().is_empty());
        session.finish(id, Some("Answer [1]".into()));
        assert_eq!(session.sources(), sources);
        let follow_up = session
            .begin(identity(1, 2), None, Some("Compare that".into()))
            .unwrap();
        assert!(follow_up[1].content.contains("Retrieved source evidence"));
        session
            .ground_pending(
                identity(1, 2),
                follow_up,
                vec![crate::research::Source {
                    id: 2,
                    title: "Discarded".into(),
                    ..sources[0].clone()
                }],
            )
            .unwrap();
        session.finish(identity(1, 2), None);
        assert_eq!(session.sources(), sources);
        session.reset(2);
        assert!(session.sources().is_empty());
        assert!(session.ground_pending(id, initial(), Vec::new()).is_err());
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
