//! Jev Decision multi-tier failover and candidate routing strategies.

use super::accounts::{JevAccountPool, OpenRouterAccount};
use crate::config::Config;
use crate::http_provider::FailoverCandidate;
use std::sync::atomic::Ordering;
use std::time::Instant;

pub const DEFAULT_FALLBACK_DECISION_MODELS: &[&str] =
    &["baseten/mammoth-1:free", "nace-ai/drex-max"];

pub fn effective_fallback_models(config: &Config) -> Vec<String> {
    if !config.jev_openrouter_fallback_models.is_empty() {
        config.jev_openrouter_fallback_models.clone()
    } else {
        DEFAULT_FALLBACK_DECISION_MODELS
            .iter()
            .map(ToString::to_string)
            .collect()
    }
}

impl JevAccountPool {
    pub fn rotated_fallback_models(&self, config: &Config) -> Vec<String> {
        let base_models = effective_fallback_models(config);
        let m_len = base_models.len();
        if m_len <= 1 {
            return base_models;
        }
        let rot = self
            .openrouter_different_counter
            .fetch_add(1, Ordering::Relaxed);
        let offset = rot % m_len;
        (0..m_len)
            .map(|i| base_models[(offset + i) % m_len].clone())
            .collect()
    }

    pub(crate) fn direct_candidates(&self) -> (Vec<FailoverCandidate>, Vec<FailoverCandidate>) {
        let direct_guard = match self.accounts.read() {
            Ok(g) => g,
            Err(_) => return (Vec::new(), Vec::new()),
        };

        let mut healthy_direct = Vec::new();
        let mut exhausted_direct = Vec::new();

        if let Ok(ex) = self.exhausted_accounts.read() {
            for acc in direct_guard.iter() {
                if ex.contains_key(&acc.api_key) {
                    exhausted_direct.push(acc.clone());
                } else {
                    healthy_direct.push(acc.clone());
                }
            }
        } else {
            healthy_direct = direct_guard.clone();
        }

        let rotated_healthy: Vec<FailoverCandidate> = if healthy_direct.is_empty() {
            Vec::new()
        } else {
            let rotation = self.counter.fetch_add(1, Ordering::Relaxed);
            let len = healthy_direct.len();
            let offset = rotation % len;
            (0..len)
                .map(|step| {
                    let acc = &healthy_direct[(offset + step) % len];
                    FailoverCandidate::direct(acc.api_key.clone(), acc.label.clone())
                })
                .collect()
        };

        let rotated_exhausted: Vec<FailoverCandidate> = exhausted_direct
            .into_iter()
            .map(|acc| FailoverCandidate::direct(acc.api_key, acc.label))
            .collect();

        (rotated_healthy, rotated_exhausted)
    }

    pub(crate) fn openrouter_candidates(
        &self,
        config: &Config,
    ) -> (
        Vec<FailoverCandidate>,
        Vec<FailoverCandidate>,
        Vec<FailoverCandidate>,
        Vec<FailoverCandidate>,
    ) {
        if !config.jev_openrouter_enabled {
            return (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        }

        let or_guard = self
            .openrouter_accounts
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let cfg_api_key = config.jev_openrouter_api_key.trim();
        let cfg_cookie = config.jev_openrouter_cookie.trim();

        let mut all_or = Vec::with_capacity(or_guard.len() + 2);
        if !cfg_api_key.is_empty() && !or_guard.iter().any(|a| a.credential() == cfg_api_key) {
            all_or.push(OpenRouterAccount {
                cookie: String::new(),
                api_key: cfg_api_key.to_string(),
                label: "openrouter-api-key".to_string(),
            });
        }
        all_or.extend(or_guard.iter().cloned());
        if !cfg_cookie.is_empty() && !all_or.iter().any(|a| a.credential() == cfg_cookie) {
            all_or.push(OpenRouterAccount {
                cookie: cfg_cookie.to_string(),
                api_key: String::new(),
                label: "openrouter-cookie".to_string(),
            });
        }

        let mut healthy_keys = Vec::new();
        let mut healthy_cookies = Vec::new();
        let mut exhausted_or = Vec::new();

        for acc in all_or {
            let cred = acc.credential();
            let is_exhausted = if let Ok(ref ex) = self.exhausted_accounts.read() {
                ex.contains_key(cred)
            } else {
                false
            };

            if is_exhausted {
                exhausted_or.push(acc);
            } else if !acc.api_key.is_empty() {
                healthy_keys.push(acc);
            } else if !acc.cookie.is_empty() {
                healthy_cookies.push(acc);
            }
        }

        let mut or_key_candidates = Vec::new();
        let mut or_alt_candidates = Vec::new();
        if !healthy_keys.is_empty() {
            let rotation = self.openrouter_counter.fetch_add(1, Ordering::Relaxed);
            let len = healthy_keys.len();
            let offset = rotation % len;
            for step in 0..len {
                let acc = &healthy_keys[(offset + step) % len];
                or_key_candidates.push(FailoverCandidate::openrouter_api_key(
                    acc.api_key.clone(),
                    acc.label.clone(),
                    None,
                ));
            }

            let alt_models = self.rotated_fallback_models(config);
            for model in alt_models {
                for step in 0..len {
                    let acc = &healthy_keys[(offset + step) % len];
                    or_alt_candidates.push(FailoverCandidate::openrouter_api_key(
                        acc.api_key.clone(),
                        format!("{}-different", acc.label),
                        Some(model.clone()),
                    ));
                }
            }
        }

        let mut or_browser_candidates = Vec::new();
        if !healthy_cookies.is_empty() {
            let rotation = self
                .openrouter_browser_counter
                .fetch_add(1, Ordering::Relaxed);
            let len = healthy_cookies.len();
            let offset = rotation % len;
            for step in 0..len {
                let acc = &healthy_cookies[(offset + step) % len];
                or_browser_candidates.push(FailoverCandidate::openrouter_browser(
                    acc.cookie.clone(),
                    acc.label.clone(),
                    None,
                ));
            }
        }

        let rotated_exhausted_or: Vec<FailoverCandidate> = exhausted_or
            .into_iter()
            .map(|acc| {
                if !acc.api_key.is_empty() {
                    FailoverCandidate::openrouter_api_key(acc.api_key, acc.label, None)
                } else {
                    FailoverCandidate::openrouter_browser(acc.cookie, acc.label, None)
                }
            })
            .collect();

        (
            or_key_candidates,
            or_alt_candidates,
            or_browser_candidates,
            rotated_exhausted_or,
        )
    }

    pub fn candidate_failover_targets(&self, config: &Config) -> Vec<FailoverCandidate> {
        let now = Instant::now();
        if let Ok(mut ex) = self.exhausted_accounts.write() {
            ex.retain(|_, expiry| *expiry > now);
        }

        let (healthy_direct, exhausted_direct) = self.direct_candidates();
        let (or_keys, or_alts, or_browsers, exhausted_or) = self.openrouter_candidates(config);

        let mut targets = Vec::new();
        targets.extend(healthy_direct);
        targets.extend(or_keys);
        targets.extend(or_alts);
        targets.extend(or_browsers);
        targets.extend(exhausted_direct);
        targets.extend(exhausted_or);
        targets
    }
}
