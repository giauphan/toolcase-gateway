use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JevAccount {
    pub api_key: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct JevAccountPool {
    accounts: Vec<JevAccount>,
    counter: Arc<AtomicUsize>,
}

impl JevAccountPool {
    pub fn new(keys: &[String]) -> Self {
        let accounts: Vec<JevAccount> = keys
            .iter()
            .enumerate()
            .map(|(idx, k)| {
                let trimmed = k.trim().to_string();
                let label = if trimmed.len() > 8 {
                    format!("account-{} (..{})", idx + 1, &trimmed[trimmed.len() - 4..])
                } else {
                    format!("account-{}", idx + 1)
                };
                JevAccount {
                    api_key: trimmed,
                    label,
                }
            })
            .filter(|acct| !acct.api_key.is_empty())
            .collect();

        Self {
            accounts,
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    pub fn next_account(&self) -> Option<JevAccount> {
        if self.accounts.is_empty() {
            return None;
        }
        let index = self.counter.fetch_add(1, Ordering::Relaxed) % self.accounts.len();
        self.accounts.get(index).cloned()
    }

    pub fn candidate_accounts(&self) -> Vec<JevAccount> {
        if self.accounts.is_empty() {
            return Vec::new();
        }
        let rotation = self.counter.fetch_add(1, Ordering::Relaxed);
        let len = self.accounts.len();
        let offset = rotation % len;
        (0..len)
            .map(|step| self.accounts[(offset + step) % len].clone())
            .collect()
    }
}
