use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JevAccount {
    pub api_key: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenRouterAccount {
    #[serde(default)]
    pub cookie: String,
    #[serde(default)]
    pub api_key: String,
    pub label: String,
}

impl OpenRouterAccount {
    pub fn credential(&self) -> &str {
        if !self.api_key.trim().is_empty() {
            self.api_key.trim()
        } else {
            self.cookie.trim()
        }
    }
}

#[derive(Clone, Debug)]
pub struct JevAccountPool {
    pub(crate) accounts: Arc<RwLock<Vec<JevAccount>>>,
    pub(crate) counter: Arc<AtomicUsize>,
    pub(crate) openrouter_accounts: Arc<RwLock<Vec<OpenRouterAccount>>>,
    pub(crate) openrouter_counter: Arc<AtomicUsize>,
    pub(crate) openrouter_browser_counter: Arc<AtomicUsize>,
    pub(crate) openrouter_different_counter: Arc<AtomicUsize>,
    pub(crate) exhausted_accounts: Arc<RwLock<HashMap<String, Instant>>>,
    pub(crate) credit_cooldown_secs: u64,
}

pub use super::failover::*;

impl JevAccountPool {
    pub fn new(keys: &[String]) -> Self {
        let accounts: Vec<JevAccount> = Self::build_accounts(keys, 0);
        Self::new_with_all(accounts, Vec::new(), 300)
    }

    pub fn new_with_accounts(accounts: Vec<JevAccount>) -> Self {
        Self::new_with_all(accounts, Vec::new(), 300)
    }

    pub fn new_with_all(
        accounts: Vec<JevAccount>,
        openrouter_accounts: Vec<OpenRouterAccount>,
        credit_cooldown_secs: u64,
    ) -> Self {
        let filtered: Vec<JevAccount> = accounts
            .into_iter()
            .map(|mut a| {
                a.api_key = a.api_key.trim().to_string();
                a.label = a.label.trim().to_string();
                a
            })
            .filter(|a| !a.api_key.is_empty())
            .collect();

        let filtered_or: Vec<OpenRouterAccount> = openrouter_accounts
            .into_iter()
            .map(|mut a| {
                a.cookie = a.cookie.trim().to_string();
                a.api_key = a.api_key.trim().to_string();
                a.label = a.label.trim().to_string();
                a
            })
            .filter(|a| !a.cookie.is_empty() || !a.api_key.is_empty())
            .collect();

        Self {
            accounts: Arc::new(RwLock::new(filtered)),
            counter: Arc::new(AtomicUsize::new(0)),
            openrouter_accounts: Arc::new(RwLock::new(filtered_or)),
            openrouter_counter: Arc::new(AtomicUsize::new(0)),
            openrouter_browser_counter: Arc::new(AtomicUsize::new(0)),
            openrouter_different_counter: Arc::new(AtomicUsize::new(0)),
            exhausted_accounts: Arc::new(RwLock::new(HashMap::new())),
            credit_cooldown_secs: if credit_cooldown_secs == 0 {
                300
            } else {
                credit_cooldown_secs
            },
        }
    }

    fn build_accounts(keys: &[String], start_idx: usize) -> Vec<JevAccount> {
        let mut accounts = Vec::new();
        for k in keys {
            let (custom_label, key_str) = parse_entry_label(k, None);
            if key_str.is_empty() {
                continue;
            }
            let num = start_idx + accounts.len() + 1;
            let label = custom_label.unwrap_or_else(|| auto_label("account", num, &key_str));
            accounts.push(JevAccount {
                api_key: key_str,
                label,
            });
        }
        accounts
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0 && self.openrouter_len() == 0
    }

    pub fn len(&self) -> usize {
        self.accounts.read().map(|a| a.len()).unwrap_or(0)
    }

    pub fn openrouter_len(&self) -> usize {
        self.openrouter_accounts
            .read()
            .map(|a| a.len())
            .unwrap_or(0)
    }

    pub fn next_account(&self) -> Option<JevAccount> {
        let guard = self.accounts.read().ok()?;
        if guard.is_empty() {
            return None;
        }
        let index = self.counter.fetch_add(1, Ordering::Relaxed) % guard.len();
        guard.get(index).cloned()
    }

    pub fn candidate_accounts(&self) -> Vec<JevAccount> {
        let guard = match self.accounts.read() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        if guard.is_empty() {
            return Vec::new();
        }
        let rotation = self.counter.fetch_add(1, Ordering::Relaxed);
        let len = guard.len();
        let offset = rotation % len;
        (0..len)
            .map(|step| guard[(offset + step) % len].clone())
            .collect()
    }

    pub fn mark_credit_exhausted(&self, key: &str) {
        let clean = key.trim();
        if clean.is_empty() {
            return;
        }
        let expiry = Instant::now() + Duration::from_secs(self.credit_cooldown_secs);
        if let Ok(mut guard) = self.exhausted_accounts.write() {
            guard.insert(clean.to_string(), expiry);
        }
    }

    pub fn clear_credit_exhausted(&self, key: &str) {
        let clean = key.trim();
        if let Ok(mut guard) = self.exhausted_accounts.write() {
            guard.remove(clean);
        }
    }

    pub fn reset_credit_exhausted(&self) {
        if let Ok(mut guard) = self.exhausted_accounts.write() {
            guard.clear();
        }
    }

    pub fn is_credit_exhausted(&self, key: &str) -> bool {
        let clean = key.trim();
        let now = Instant::now();
        if let Ok(guard) = self.exhausted_accounts.read() {
            if let Some(expiry) = guard.get(clean) {
                return *expiry > now;
            }
        }
        false
    }

    pub fn exhausted_accounts_count(&self) -> usize {
        let now = Instant::now();
        if let Ok(guard) = self.exhausted_accounts.read() {
            guard.values().filter(|expiry| **expiry > now).count()
        } else {
            0
        }
    }

    pub fn add_keys(&self, keys: &[String]) -> usize {
        let entries: Vec<(String, Option<String>)> =
            keys.iter().map(|k| (k.clone(), None)).collect();
        self.add_account_entries(&entries)
    }

    pub fn add_account_entries(&self, entries: &[(String, Option<String>)]) -> usize {
        let mut guard = match self.accounts.write() {
            Ok(g) => g,
            Err(_) => return 0,
        };
        let mut added = 0;
        for (raw_key, custom_label) in entries {
            let (parsed_label, trimmed_key) = parse_entry_label(raw_key, custom_label.clone());
            if trimmed_key.is_empty() {
                continue;
            }

            if let Some(existing) = guard.iter_mut().find(|a| a.api_key == trimmed_key) {
                if let Some(ref l) = parsed_label {
                    let l_trim = l.trim();
                    if !l_trim.is_empty() {
                        existing.label = l_trim.to_string();
                    }
                }
                continue;
            }

            let num = guard.len() + 1;
            let label = parsed_label
                .filter(|l| !l.trim().is_empty())
                .unwrap_or_else(|| auto_label("account", num, &trimmed_key));

            guard.push(JevAccount {
                api_key: trimmed_key,
                label,
            });
            added += 1;
        }
        added
    }

    pub fn add_openrouter_keys(&self, cookies: &[String]) -> usize {
        let entries: Vec<(String, Option<String>)> =
            cookies.iter().map(|c| (c.clone(), None)).collect();
        self.add_openrouter_entries(&entries)
    }

    pub fn add_openrouter_entries(&self, entries: &[(String, Option<String>)]) -> usize {
        self.add_openrouter_typed_entries(entries, None)
    }

    pub fn add_openrouter_typed_entries(
        &self,
        entries: &[(String, Option<String>)],
        force_type: Option<bool>,
    ) -> usize {
        let mut guard = match self.openrouter_accounts.write() {
            Ok(g) => g,
            Err(_) => return 0,
        };
        let mut added = 0;
        for (raw_str, custom_label) in entries {
            let (parsed_label, trimmed_str) = parse_entry_label(raw_str, custom_label.clone());
            if trimmed_str.is_empty() {
                continue;
            }

            if let Some(existing) = guard.iter_mut().find(|a| a.credential() == trimmed_str) {
                if let Some(ref l) = parsed_label {
                    let l_trim = l.trim();
                    if !l_trim.is_empty() {
                        existing.label = l_trim.to_string();
                    }
                }
                continue;
            }

            let is_key = force_type.unwrap_or_else(|| {
                crate::providers::jev::openrouter::is_openrouter_api_key(&trimmed_str)
            });
            let num = guard.len() + 1;
            let label_prefix = if is_key {
                "openrouter-key"
            } else {
                "openrouter"
            };
            let label = parsed_label
                .filter(|l| !l.trim().is_empty())
                .unwrap_or_else(|| auto_label(label_prefix, num, &trimmed_str));

            let (api_key, cookie) = if is_key {
                (trimmed_str, String::new())
            } else {
                (String::new(), trimmed_str)
            };

            guard.push(OpenRouterAccount {
                cookie,
                api_key,
                label,
            });
            added += 1;
        }
        added
    }

    pub fn accounts(&self) -> Vec<JevAccount> {
        self.accounts.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn openrouter_accounts(&self) -> Vec<OpenRouterAccount> {
        self.openrouter_accounts
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    pub fn keys(&self) -> Vec<String> {
        self.accounts
            .read()
            .map(|g| g.iter().map(|a| a.api_key.clone()).collect())
            .unwrap_or_default()
    }

    pub fn openrouter_keys(&self) -> Vec<String> {
        self.openrouter_accounts
            .read()
            .map(|g| g.iter().map(|a| a.credential().to_string()).collect())
            .unwrap_or_default()
    }

    pub fn openrouter_cookies(&self) -> Vec<String> {
        self.openrouter_accounts
            .read()
            .map(|g| g.iter().map(|a| a.cookie.clone()).collect())
            .unwrap_or_default()
    }
}

pub(crate) fn parse_entry_label(raw: &str, custom: Option<String>) -> (Option<String>, String) {
    let trimmed = raw.trim();
    if let Some(c) = custom {
        let c_trim = c.trim();
        if !c_trim.is_empty() {
            return (Some(c_trim.to_string()), trimmed.to_string());
        }
    }
    if let Some((lbl, part)) = trimmed.split_once(':') {
        let l = lbl.trim();
        let p = part.trim();
        if !l.is_empty() && !p.is_empty() && !l.contains(';') && !l.contains('=') {
            return (Some(l.to_string()), p.to_string());
        }
    }
    if let Some((lbl, part)) = trimmed.split_once('=') {
        let l = lbl.trim();
        let p = part.trim();
        if !l.is_empty() && !p.is_empty() && !p.contains(';') && p.starts_with("sk-") {
            return (Some(l.to_string()), p.to_string());
        }
    }
    (None, trimmed.to_string())
}

fn auto_label(prefix: &str, num: usize, val: &str) -> String {
    if val.len() > 8 {
        format!("{prefix}-{num} (..{})", &val[val.len() - 4..])
    } else {
        format!("{prefix}-{num}")
    }
}
