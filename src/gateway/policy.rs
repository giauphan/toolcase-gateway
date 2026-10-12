use crate::config::{Config, RoutingMode};
use crate::gateway::capabilities::{lookup_capabilities, select_model_by_tier};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingDecision {
    pub selected_model: String,
    pub candidates: Vec<String>,
    pub strategy: RoutingMode,
    pub reason: String,
    pub shadow_recommendation: Option<String>,
}

pub fn evaluate_routing_policy(
    requested_model: &str,
    candidates: Vec<String>,
    config: &Config,
    has_tools: bool,
) -> RoutingDecision {
    match config.routing_mode {
        RoutingMode::Existing => RoutingDecision {
            selected_model: requested_model.to_string(),
            candidates,
            strategy: RoutingMode::Existing,
            reason: "configured existing passthrough".to_string(),
            shadow_recommendation: evaluate_shadow_mode(requested_model, config, has_tools),
        },
        RoutingMode::RoundRobin => {
            let selected = candidates
                .first()
                .cloned()
                .unwrap_or_else(|| requested_model.to_string());
            RoutingDecision {
                selected_model: selected,
                candidates,
                strategy: RoutingMode::RoundRobin,
                reason: "round_robin candidate rotation".to_string(),
                shadow_recommendation: evaluate_shadow_mode(requested_model, config, has_tools),
            }
        }
        RoutingMode::Rules => {
            let tier = if has_tools {
                crate::config::ModelTier::Strong
            } else {
                config.default_tier
            };
            let recommended = select_model_by_tier(tier, requested_model);
            let mut final_candidates = vec![recommended.clone()];
            for c in candidates {
                if !final_candidates.contains(&c) {
                    final_candidates.push(c);
                }
            }
            RoutingDecision {
                selected_model: recommended,
                candidates: final_candidates,
                strategy: RoutingMode::Rules,
                reason: format!("deterministic rules matched tier {:?}", tier),
                shadow_recommendation: None,
            }
        }
        RoutingMode::Jev => {
            let recommended = evaluate_shadow_mode(requested_model, config, has_tools)
                .unwrap_or_else(|| requested_model.to_string());
            let mut final_candidates = vec![recommended.clone()];
            for c in candidates {
                if !final_candidates.contains(&c) {
                    final_candidates.push(c);
                }
            }
            RoutingDecision {
                selected_model: recommended,
                candidates: final_candidates,
                strategy: RoutingMode::Jev,
                reason: "jev decision engine".to_string(),
                shadow_recommendation: None,
            }
        }
    }
}

fn evaluate_shadow_mode(requested_model: &str, config: &Config, has_tools: bool) -> Option<String> {
    if !config.jev_shadow_mode {
        return None;
    }
    let caps = lookup_capabilities(requested_model);
    if has_tools && caps.is_some_and(|c| !c.supports_tools) {
        Some("claude-3-5-sonnet".to_string())
    } else {
        Some(select_model_by_tier(config.default_tier, requested_model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelTier;

    #[test]
    fn test_evaluate_routing_policy_existing_with_shadow() {
        let cfg = Config {
            routing_mode: RoutingMode::Existing,
            jev_shadow_mode: true,
            default_tier: ModelTier::Fast,
            ..Config::default()
        };

        let decision = evaluate_routing_policy(
            "custom-model",
            vec!["custom-model".to_string(), "fail-try".to_string()],
            &cfg,
            false,
        );
        assert_eq!(decision.strategy, RoutingMode::Existing);
        assert_eq!(decision.selected_model, "custom-model");
        assert_eq!(
            decision.shadow_recommendation.as_deref(),
            Some("claude-3-5-haiku")
        );
    }

    #[test]
    fn test_evaluate_routing_policy_rules_tier_strong_on_tools() {
        let cfg = Config {
            routing_mode: RoutingMode::Rules,
            default_tier: ModelTier::Fast,
            ..Config::default()
        };

        let decision = evaluate_routing_policy(
            "custom-model",
            vec!["custom-model".to_string(), "fail-try".to_string()],
            &cfg,
            true,
        );
        assert_eq!(decision.strategy, RoutingMode::Rules);
        assert_eq!(decision.selected_model, "claude-3-5-sonnet");
    }
}
