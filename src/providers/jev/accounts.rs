use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    pub fn new_with_accounts(accounts: Vec<JevAccount>) -> Self {
        let filtered: Vec<JevAccount> = accounts
            .into_iter()
            .map(|mut a| {
                a.api_key = a.api_key.trim().to_string();
                a.label = a.label.trim().to_string();
                a
            })
            .filter(|a| !a.api_key.is_empty())
            .collect();
        Self {
            accounts: Arc::new(RwLock::new(filtered)),
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn build_accounts(keys: &[String], start_idx: usize) -> Vec<JevAccount> {
        let mut accounts = Vec::new();
        for k in keys {
            let trimmed = k.trim();
            if trimmed.is_empty() {
                continue;
            }
            let (custom_label, key_str) = if let Some((lbl, k_part)) =
                trimmed.split_once(':').or_else(|| trimmed.split_once('='))
            {
                let l = lbl.trim();
                let kp = k_part.trim();
                if !l.is_empty() && !kp.is_empty() {
                    (Some(l.to_string()), kp.to_string())
                } else {
                    (None, trimmed.to_string())
                }
            } else {
                (None, trimmed.to_string())
            };

            let num = start_idx + accounts.len() + 1;
            let label = match custom_label {
                Some(lbl) => lbl,
                None => {
                    if key_str.len() > 8 {
                        format!("account-{} (..{})", num, &key_str[key_str.len() - 4..])
                    } else {
                        format!("account-{}", num)
                    }
                }
            };
            accounts.push(JevAccount {
                api_key: key_str,
                label,
            });
        }
        accounts
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
            let (parsed_label, trimmed_key) = if let Some((lbl, kp)) =
                raw_key.split_once(':').or_else(|| raw_key.split_once('='))
            {
                let l = lbl.trim();
                let kp = kp.trim();
                if !l.is_empty() && !kp.is_empty() {
                    (Some(l.to_string()), kp.to_string())
                } else {
                    (custom_label.clone(), raw_key.trim().to_string())
                }
            } else {
                (custom_label.clone(), raw_key.trim().to_string())
            };

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
            let label = match parsed_label {
                Some(ref l) if !l.trim().is_empty() => l.trim().to_string(),
                _ => {
                    if trimmed_key.len() > 8 {
                        format!(
                            "account-{} (..{})",
                            num,
                            &trimmed_key[trimmed_key.len() - 4..]
                        )
                    } else {
                        format!("account-{}", num)
                    }
                }
            };

            guard.push(JevAccount {
                api_key: trimmed_key,
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
