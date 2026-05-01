//! Capability lookup — a *hint* for unknown routes, not an
//! enforcement boundary.
//!
//! Per `dev-docs/templates-implementation-plan.md` §5.1: this table
//! is the default fallback when a `Route` has no
//! `capabilities_override` set. The override is the actual
//! enforcement boundary; the table only provides a sane default at
//! route-creation time so the user has something to confirm.
//!
//! Algorithm: longest-prefix match against the model name. Unknown
//! models return the empty set (never assumed-capable).

use std::collections::HashSet;

use super::blueprint::Capability;

/// Convenience: a `HashSet<Capability>` with the standard derive
/// surface. Wrapped in a newtype so `From<&[Capability]>` exists
/// without conflicting with stdlib impls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySet(pub HashSet<Capability>);

impl CapabilitySet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn collect_from<I: IntoIterator<Item = Capability>>(items: I) -> Self {
        Self(items.into_iter().collect())
    }

    pub fn contains(&self, cap: &Capability) -> bool {
        self.0.contains(cap)
    }

    pub fn contains_all(&self, required: &[Capability]) -> bool {
        required.iter().all(|c| self.0.contains(c))
    }

    pub fn missing<'a>(&self, required: &'a [Capability]) -> Vec<&'a Capability> {
        required.iter().filter(|c| !self.0.contains(c)).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The default-capability table. Each row is `(model-name prefix,
/// capabilities)`. Longest prefix wins. Edits here are
/// best-effort; new model families just need a row.
const TABLE: &[(&str, &[Capability])] = &[
    // Anthropic — direct + Bedrock + Vertex naming.
    (
        "claude-",
        &[
            Capability::ToolUse,
            Capability::LongContext,
            Capability::Vision,
            Capability::StructuredOutput,
        ],
    ),
    (
        "anthropic.claude-",
        &[
            Capability::ToolUse,
            Capability::LongContext,
            Capability::Vision,
            Capability::StructuredOutput,
        ],
    ),
    (
        "us.anthropic.claude-",
        &[
            Capability::ToolUse,
            Capability::LongContext,
            Capability::Vision,
            Capability::StructuredOutput,
        ],
    ),
    (
        "publishers/anthropic/",
        &[
            Capability::ToolUse,
            Capability::LongContext,
            Capability::Vision,
            Capability::StructuredOutput,
        ],
    ),
    // OpenAI.
    (
        "gpt-4",
        &[
            Capability::ToolUse,
            Capability::Vision,
            Capability::StructuredOutput,
        ],
    ),
    (
        "gpt-5",
        &[
            Capability::ToolUse,
            Capability::Vision,
            Capability::StructuredOutput,
            Capability::LongContext,
        ],
    ),
    ("o1-", &[Capability::StructuredOutput]),
    // Open weights via Ollama / LM Studio / vLLM. The vision rows
    // must come *after* their non-vision base so longest-prefix
    // selects them.
    ("llama-3.1-", &[Capability::ToolUse]),
    ("llama-3.2-", &[Capability::ToolUse, Capability::Vision]),
    (
        "qwen2.5-",
        &[Capability::ToolUse, Capability::StructuredOutput],
    ),
    (
        "qwen2.5-vl-",
        &[
            Capability::ToolUse,
            Capability::StructuredOutput,
            Capability::Vision,
        ],
    ),
    ("phi-3", &[]),
    ("mistral-", &[Capability::ToolUse]),
    (
        "deepseek-v3",
        &[Capability::ToolUse, Capability::StructuredOutput],
    ),
];

/// Default capability set for a model name, by longest-prefix
/// match. Returns empty for unknown models — never assumed-capable.
pub fn default_capabilities_for(model: &str) -> CapabilitySet {
    let lower = model.to_ascii_lowercase();
    let best = TABLE
        .iter()
        .filter(|(prefix, _)| lower.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len());
    match best {
        Some((_, caps)) => CapabilitySet::collect_from(caps.iter().copied()),
        None => CapabilitySet::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_beats_short() {
        // Both "qwen2.5-" and "qwen2.5-vl-" match; the vl row wins
        // because its prefix is longer.
        let caps = default_capabilities_for("qwen2.5-vl-7b-instruct");
        assert!(caps.contains(&Capability::Vision));
        assert!(caps.contains(&Capability::ToolUse));
    }

    #[test]
    fn shorter_prefix_used_when_longer_does_not_match() {
        let caps = default_capabilities_for("qwen2.5-7b-instruct");
        assert!(caps.contains(&Capability::ToolUse));
        assert!(caps.contains(&Capability::StructuredOutput));
        // Vision is a property of the -vl- variant only.
        assert!(!caps.contains(&Capability::Vision));
    }

    #[test]
    fn unknown_model_is_empty() {
        let caps = default_capabilities_for("totally-unknown-model-2027");
        assert!(caps.is_empty());
    }

    #[test]
    fn case_insensitive() {
        let lower = default_capabilities_for("claude-haiku-4-5");
        let upper = default_capabilities_for("CLAUDE-HAIKU-4-5");
        assert_eq!(lower.0, upper.0);
    }

    #[test]
    fn anthropic_via_bedrock_aliases() {
        for alias in [
            "anthropic.claude-3-5-sonnet-20240620-v1:0",
            "us.anthropic.claude-3-5-sonnet-20240620-v1:0",
        ] {
            let caps = default_capabilities_for(alias);
            assert!(caps.contains(&Capability::ToolUse), "alias {alias} should have tool use");
            assert!(caps.contains(&Capability::Vision), "alias {alias} should have vision");
        }
    }

    #[test]
    fn vertex_publishers_path_recognized() {
        let caps = default_capabilities_for("publishers/anthropic/models/claude-sonnet");
        assert!(caps.contains(&Capability::ToolUse));
    }

    #[test]
    fn contains_all_helper() {
        let caps = default_capabilities_for("claude-haiku-4-5");
        assert!(caps.contains_all(&[Capability::ToolUse]));
        assert!(caps.contains_all(&[Capability::ToolUse, Capability::Vision]));
        // A vision-less model fails the all-of check.
        let phi = default_capabilities_for("phi-3-mini");
        assert!(!phi.contains_all(&[Capability::ToolUse]));
    }

    #[test]
    fn missing_helper_lists_gaps() {
        let phi = default_capabilities_for("phi-3-mini");
        let gaps = phi.missing(&[Capability::ToolUse, Capability::Vision]);
        assert_eq!(gaps.len(), 2);
    }
}
