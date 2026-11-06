//! In-flight module upgrade registry to enforce serialization and prevent duplicate concurrent upgrades.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Default maximum time-to-live for an in-flight upgrade lock before auto-eviction (5 minutes).
pub const DEFAULT_UPGRADE_LOCK_TTL: Duration = Duration::from_secs(300);

static UPGRADING_MODULES: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

/// RAII guard that automatically removes a module from the in-flight upgrade set when dropped.
#[derive(Debug)]
pub struct UpgradeGuard {
    module_id: String,
    acquired_at: Instant,
}

impl Drop for UpgradeGuard {
    fn drop(&mut self) {
        let mut guard = UPGRADING_MODULES
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(map) = guard.as_mut() {
            if let Some(&ts) = map.get(&self.module_id) {
                if ts == self.acquired_at {
                    map.remove(&self.module_id);
                }
            }
        }
    }
}

/// Try to acquire an exclusive in-flight upgrade slot for the given module ID with a custom TTL.
/// Returns an `UpgradeGuard` on success, or an error if the module is already upgrading and the lock has not expired.
pub fn try_acquire_upgrade_with_ttl(
    module_id: &str,
    ttl: Duration,
) -> Result<UpgradeGuard, String> {
    let clean_id = module_id.trim().to_string();
    let mut guard = UPGRADING_MODULES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let map = guard.get_or_insert_with(HashMap::new);

    if let Some(&start_time) = map.get(&clean_id) {
        if start_time.elapsed() < ttl {
            return Err(format!(
                "Module '{clean_id}' is currently being upgraded. Please wait."
            ));
        }
        // Stale lock detected; evict and reclaim slot
        map.remove(&clean_id);
    }

    let now = Instant::now();
    map.insert(clean_id.clone(), now);
    Ok(UpgradeGuard {
        module_id: clean_id,
        acquired_at: now,
    })
}

/// Try to acquire an exclusive in-flight upgrade slot for the given module ID using default TTL.
/// Returns an `UpgradeGuard` on success, or an error if the module is already upgrading.
pub fn try_acquire_upgrade(module_id: &str) -> Result<UpgradeGuard, String> {
    try_acquire_upgrade_with_ttl(module_id, DEFAULT_UPGRADE_LOCK_TTL)
}

/// Clears all active upgrade locks. Primarily intended for testing and state resets.
pub fn clear_all_upgrade_locks() {
    let mut guard = UPGRADING_MODULES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(map) = guard.as_mut() {
        map.clear();
    }
}

/// Returns a sorted list of module IDs currently undergoing upgrades, pruning any stale locks.
pub fn get_in_flight_upgrades() -> Vec<String> {
    let mut guard = UPGRADING_MODULES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(map) = guard.as_mut() {
        map.retain(|_id, start_time| start_time.elapsed() < DEFAULT_UPGRADE_LOCK_TTL);
        let mut list: Vec<String> = map.keys().cloned().collect();
        list.sort();
        return list;
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_acquire_upgrade_lifecycle() {
        let test_id = "test-guard-mod";
        clear_all_upgrade_locks();
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

    #[test]
    fn test_stale_upgrade_lock_expiration() {
        let test_id = "test-stale-lock-mod";
        clear_all_upgrade_locks();

        // Acquire with a very short TTL
        let short_ttl = Duration::from_millis(50);
        let _guard = try_acquire_upgrade_with_ttl(test_id, short_ttl).expect("acquire succeeds");

        // Concurrent acquire immediately after fails
        assert!(try_acquire_upgrade_with_ttl(test_id, short_ttl).is_err());

        // Wait for TTL to expire
        std::thread::sleep(Duration::from_millis(70));

        // Re-acquiring should now succeed because the previous lock has expired and is evicted
        let guard2 = try_acquire_upgrade_with_ttl(test_id, short_ttl)
            .expect("re-acquire succeeds after TTL expiration");
        drop(guard2);
        assert!(!get_in_flight_upgrades().contains(&test_id.to_string()));
    }

    #[test]
    fn test_expired_guard_drop_does_not_evict_newer_lock() {
        let test_id = "test-aba-lock-mod";
        clear_all_upgrade_locks();

        let short_ttl = Duration::from_millis(50);
        let guard1 = try_acquire_upgrade_with_ttl(test_id, short_ttl).expect("guard1 succeeds");

        // Wait past TTL
        std::thread::sleep(Duration::from_millis(70));

        // guard2 reclaims the slot
        let guard2 = try_acquire_upgrade_with_ttl(test_id, short_ttl).expect("guard2 succeeds");
        assert!(get_in_flight_upgrades().contains(&test_id.to_string()));

        // Old guard1 drops; must NOT evict guard2's lock!
        drop(guard1);
        assert!(
            get_in_flight_upgrades().contains(&test_id.to_string()),
            "guard1 drop must not remove guard2's active lock"
        );

        // Third acquire still fails while guard2 is active
        assert!(try_acquire_upgrade_with_ttl(test_id, short_ttl).is_err());

        // Now drop guard2; slot is freed
        drop(guard2);
        assert!(!get_in_flight_upgrades().contains(&test_id.to_string()));
    }
}
