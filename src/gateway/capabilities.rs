use crate::config::ModelTier;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCapabilities {
    pub model_id: &'static str,
    pub tier: ModelTier,
    pub context_window: usize,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
}

pub const KNOWN_MODELS: &[ModelCapabilities] = &[
    ModelCapabilities {
        model_id: "claude-3-5-sonnet",
        tier: ModelTier::Strong,
        context_window: 200_000,
        supports_tools: true,
        supports_reasoning: true,
    },
    ModelCapabilities {
        model_id: "claude-3-5-haiku",
        tier: ModelTier::Fast,
        context_window: 200_000,
        supports_tools: true,
        supports_reasoning: false,
    },
    ModelCapabilities {
        model_id: "gpt-4o",
        tier: ModelTier::Strong,
        context_window: 128_000,
        supports_tools: true,
        supports_reasoning: true,
    },
    ModelCapabilities {
        model_id: "gpt-4o-mini",
        tier: ModelTier::Fast,
        context_window: 128_000,
        supports_tools: true,
        supports_reasoning: false,
    },
    ModelCapabilities {
        model_id: "muse",
        tier: ModelTier::Strong,
        context_window: 128_000,
        supports_tools: true,
        supports_reasoning: true,
    },
];

pub fn lookup_capabilities(model: &str) -> Option<&'static ModelCapabilities> {
    let clean = model.trim().to_ascii_lowercase();
    KNOWN_MODELS
        .iter()
        .find(|m| m.model_id.eq_ignore_ascii_case(&clean) || clean.starts_with(m.model_id))
}

pub fn select_model_by_tier(tier: ModelTier, fallback: &str) -> String {
    match tier {
        ModelTier::Fast => "claude-3-5-haiku".to_string(),
        ModelTier::Strong => "claude-3-5-sonnet".to_string(),
        ModelTier::Unknown => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_capabilities() {
        let sonnet = lookup_capabilities("claude-3-5-sonnet-20241022").unwrap();
        assert_eq!(sonnet.tier, ModelTier::Strong);
        assert!(sonnet.supports_tools);

        let haiku = lookup_capabilities("claude-3-5-haiku").unwrap();
        assert_eq!(haiku.tier, ModelTier::Fast);

        assert!(lookup_capabilities("unknown-model").is_none());
    }

    #[test]
    fn test_select_model_by_tier() {
        assert_eq!(
            select_model_by_tier(ModelTier::Fast, "fb"),
            "claude-3-5-haiku"
        );
        assert_eq!(
            select_model_by_tier(ModelTier::Strong, "fb"),
            "claude-3-5-sonnet"
        );
        assert_eq!(select_model_by_tier(ModelTier::Unknown, "fb"), "fb");
    }
}
