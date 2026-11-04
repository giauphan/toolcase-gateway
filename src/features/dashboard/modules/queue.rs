//! In-flight module upgrade registry to enforce serialization and prevent duplicate concurrent upgrades.

use std::collections::HashSet;
use std::sync::Mutex;

static UPGRADING_MODULES: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// RAII guard that automatically removes a module from the in-flight upgrade set when dropped.
#[derive(Debug)]
pub struct UpgradeGuard {
    module_id: String,
}

impl Drop for UpgradeGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = UPGRADING_MODULES.lock() {
            if let Some(set) = guard.as_mut() {
                set.remove(&self.module_id);
            }
        }
    }
}

/// Try to acquire an exclusive in-flight upgrade slot for the given module ID.
/// Returns an `UpgradeGuard` on success, or an error if the module is already upgrading.
pub fn try_acquire_upgrade(module_id: &str) -> Result<UpgradeGuard, String> {
    let clean_id = module_id.trim().to_string();
    let mut guard = UPGRADING_MODULES
        .lock()
        .map_err(|e| format!("Failed to lock upgrade registry: {e}"))?;
    let set = guard.get_or_insert_with(HashSet::new);
    if set.contains(&clean_id) {
        return Err(format!(
            "Module '{clean_id}' is currently being upgraded. Please wait."
        ));
    }
    set.insert(clean_id.clone());
    Ok(UpgradeGuard {
        module_id: clean_id,
    })
}

/// Returns a sorted list of module IDs currently undergoing upgrades.
pub fn get_in_flight_upgrades() -> Vec<String> {
    if let Ok(guard) = UPGRADING_MODULES.lock() {
        if let Some(set) = guard.as_ref() {
            let mut list: Vec<String> = set.iter().cloned().collect();
            list.sort();
            return list;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_acquire_upgrade_lifecycle() {
        let test_id = "test-guard-mod";
        {
            let guard = try_acquire_upgrade(test_id).expect("first acquire succeeds");
            assert!(get_in_flight_upgrades().contains(&test_id.to_string()));

            // Second concurrent acquire for the same module must fail
            let err = try_acquire_upgrade(test_id);
            assert!(err.is_err());
            assert!(err.unwrap_err().contains("currently being upgraded"));

            drop(guard);
        }

        // After drop, should be able to acquire again
        let guard2 = try_acquire_upgrade(test_id).expect("re-acquire succeeds after drop");
        drop(guard2);
        assert!(!get_in_flight_upgrades().contains(&test_id.to_string()));
    }
}
