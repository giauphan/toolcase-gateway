use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JevAccount {
    pub api_key: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct JevAccountPool {
    accounts: Arc<RwLock<Vec<JevAccount>>>,
    counter: Arc<AtomicUsize>,
}

impl JevAccountPool {
    pub fn new(keys: &[String]) -> Self {
        let accounts: Vec<JevAccount> = Self::build_accounts(keys, 0);
        Self {
            accounts: Arc::new(RwLock::new(accounts)),
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn build_accounts(keys: &[String], start_idx: usize) -> Vec<JevAccount> {
        keys.iter()
            .enumerate()
            .map(|(idx, k)| {
                let trimmed = k.trim().to_string();
                let num = start_idx + idx + 1;
                let label = if trimmed.len() > 8 {
                    format!("account-{} (..{})", num, &trimmed[trimmed.len() - 4..])
                } else {
                    format!("account-{}", num)
                };
                JevAccount {
                    api_key: trimmed,
                    label,
                }
            })
            .filter(|acct| !acct.api_key.is_empty())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.read().map(|a| a.is_empty()).unwrap_or(true)
    }

    pub fn len(&self) -> usize {
        self.accounts.read().map(|a| a.len()).unwrap_or(0)
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

    pub fn add_keys(&self, keys: &[String]) -> usize {
        let mut guard = match self.accounts.write() {
            Ok(g) => g,
            Err(_) => return 0,
        };
        let mut added = 0;
        for k in keys {
            let trimmed = k.trim().to_string();
            if trimmed.is_empty() || guard.iter().any(|a| a.api_key == trimmed) {
                continue;
            }
            let num = guard.len() + 1;
            let label = if trimmed.len() > 8 {
                format!("account-{} (..{})", num, &trimmed[trimmed.len() - 4..])
            } else {
                format!("account-{}", num)
            };
            guard.push(JevAccount {
                api_key: trimmed,
                label,
            });
            added += 1;
        }
        added
    }

    pub fn accounts(&self) -> Vec<JevAccount> {
        self.accounts.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn keys(&self) -> Vec<String> {
        self.accounts
            .read()
            .map(|g| g.iter().map(|a| a.api_key.clone()).collect())
            .unwrap_or_default()
    }
}
