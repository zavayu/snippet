//! Build-time prompt catalog and literal, single-pass placeholder substitution.
//! Template edits require a rebuild; inference never reads prompt files from disk.
use std::{borrow::Cow, fmt};

#[derive(Clone, Copy)]
pub struct Template {
    pub name: &'static str,
    source: &'static str,
}

pub const ASSISTANT: Template = Template {
    name: "assistant",
    source: include_str!("../../prompts/assistant.md"),
};
pub const RUNTIME_CONTEXT: Template = Template {
    name: "runtime-context",
    source: include_str!("../../prompts/runtime-context.md"),
};
pub const CLOCK: Template = Template {
    name: "clock",
    source: include_str!("../../prompts/clock.md"),
};
pub const SEARCH_ENABLED: Template = Template {
    name: "capabilities/search-enabled",
    source: include_str!("../../prompts/capabilities/search-enabled.md"),
};
pub const SEARCH_DISABLED: Template = Template {
    name: "capabilities/search-disabled",
    source: include_str!("../../prompts/capabilities/search-disabled.md"),
};
pub const ATTACHED_FILES: Template = Template {
    name: "attached-files",
    source: include_str!("../../prompts/attached-files.md"),
};
pub const TOOL_LOOP: Template = Template {
    name: "tool-loop",
    source: include_str!("../../prompts/tool-loop.md"),
};
pub const SEARCH_UNAVAILABLE: Template = Template {
    name: "capabilities/search-unavailable",
    source: include_str!("../../prompts/capabilities/search-unavailable.md"),
};
pub const WEB_EVIDENCE: Template = Template {
    name: "web-evidence",
    source: include_str!("../../prompts/web-evidence.md"),
};
pub const FRESHNESS: Template = Template {
    name: "freshness",
    source: include_str!("../../prompts/freshness.md"),
};
pub const RETRIEVAL_NOTE: Template = Template {
    name: "retrieval-note",
    source: include_str!("../../prompts/retrieval-note.md"),
};
pub const OCR: Template = Template {
    name: "ocr",
    source: include_str!("../../prompts/ocr.md"),
};
pub const EXPLAIN_TEXT: Template = Template {
    name: "actions/explain-text",
    source: include_str!("../../prompts/actions/explain-text.md"),
};
pub const EXPLAIN_IMAGE: Template = Template {
    name: "actions/explain-image",
    source: include_str!("../../prompts/actions/explain-image.md"),
};
pub const SUMMARIZE: Template = Template {
    name: "actions/summarize",
    source: include_str!("../../prompts/actions/summarize.md"),
};
pub const REFINE: Template = Template {
    name: "actions/refine",
    source: include_str!("../../prompts/actions/refine.md"),
};
pub const RESEARCH: Template = Template {
    name: "actions/research",
    source: include_str!("../../prompts/actions/research.md"),
};

pub const ALL: &[Template] = &[
    ASSISTANT,
    RUNTIME_CONTEXT,
    CLOCK,
    SEARCH_ENABLED,
    SEARCH_DISABLED,
    TOOL_LOOP,
    ATTACHED_FILES,
    SEARCH_UNAVAILABLE,
    WEB_EVIDENCE,
    FRESHNESS,
    RETRIEVAL_NOTE,
    OCR,
    EXPLAIN_TEXT,
    EXPLAIN_IMAGE,
    SUMMARIZE,
    REFINE,
    RESEARCH,
];

/// Normalize checkout line endings and omit the file's final newline only.
/// Leading spaces/newlines are intentional separators in several templates.
pub fn text(template: Template) -> Cow<'static, str> {
    if template.source.contains("\r\n") {
        Cow::Owned(
            template
                .source
                .replace("\r\n", "\n")
                .trim_end_matches('\n')
                .into(),
        )
    } else {
        Cow::Borrowed(template.source.trim_end_matches('\n'))
    }
}

pub fn runtime_marker() -> &'static str {
    RUNTIME_CONTEXT
        .source
        .lines()
        .next()
        .expect("runtime template header")
}

#[derive(Debug, PartialEq, Eq)]
pub enum RenderError {
    MissingValue { template: &'static str, key: String },
    UnclosedPlaceholder { template: &'static str },
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue { template, key } => write!(
                f,
                "Missing {{{{{key}}}}} value in prompt template {template}"
            ),
            Self::UnclosedPlaceholder { template } => {
                write!(f, "Unclosed placeholder in prompt template {template}")
            }
        }
    }
}
impl std::error::Error for RenderError {}

/// Insert values literally. Placeholder-looking text in a user value is never
/// interpreted as another binding, instruction, or template expression.
pub fn render(template: Template, values: &[(&str, &str)]) -> Result<String, RenderError> {
    let source = text(template);
    let mut rest = source.as_ref();
    let mut output = String::with_capacity(rest.len());
    while let Some((before, placeholder)) = rest.split_once("{{") {
        output.push_str(before);
        let (key, after) =
            placeholder
                .split_once("}}")
                .ok_or(RenderError::UnclosedPlaceholder {
                    template: template.name,
                })?;
        let key = key.trim();
        let value = values
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| *value)
            .ok_or_else(|| RenderError::MissingValue {
                template: template.name,
                key: key.into(),
            })?;
        output.push_str(value);
        rest = after;
    }
    output.push_str(rest);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substituted_values_are_literal_and_preserve_user_whitespace() {
        let source = "  {{utc_time}}\n<script>literal text</script>  ";
        let reference = render(RESEARCH, &[("instruction", source)]).unwrap();
        assert!(reference.ends_with(source));
        assert!(reference.contains("{{utc_time}}"));
        assert!(render(CLOCK, &[("utc_time", "now")]).is_err());
        let broken = Template {
            name: "broken",
            source: "{{unclosed",
        };
        assert_eq!(
            render(broken, &[]),
            Err(RenderError::UnclosedPlaceholder { template: "broken" })
        );
    }

    #[test]
    fn catalog_templates_have_unique_names_and_known_bindings() {
        let values = [
            ("capabilities", "enabled"),
            ("clock_context", "clock"),
            ("utc_time", "2026-10-22T22:00:00Z"),
            ("local_time", "2026-10-22T17:00:00-05:00"),
            ("instruction", "task"),
            ("files", "[]"),
        ];
        let mut names = std::collections::HashSet::new();
        for template in ALL {
            assert!(names.insert(template.name));
            let rendered = render(*template, &values).unwrap();
            assert!(!rendered.is_empty(), "{}", template.name);
            assert!(!rendered.contains("{{"), "{}", template.name);
        }
    }

    #[test]
    fn windows_line_endings_do_not_change_model_input() {
        let template = Template {
            name: "line-endings",
            source: "\r\n  Text\r\n{{value}}\r\n",
        };
        assert_eq!(
            render(template, &[("value", "user\r\ntext")]).unwrap(),
            "\n  Text\nuser\r\ntext"
        );
        assert_eq!(
            runtime_marker(),
            "Snippet runtime capabilities (trusted application state):"
        );
    }
}
