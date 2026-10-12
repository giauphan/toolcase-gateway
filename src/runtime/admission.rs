use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub const DEFAULT_MAX_IN_FLIGHT_BYTES: usize = 64 * 1024 * 1024; // 64 MB

#[derive(Clone, Debug)]
pub struct AdmissionController {
    max_connections: usize,
    max_in_flight_bytes: usize,
    active_connections: Arc<AtomicUsize>,
    in_flight_bytes: Arc<AtomicUsize>,
}

pub struct ConnectionGuard {
    active_connections: Arc<AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

pub struct ByteGuard {
    amount: usize,
    in_flight_bytes: Arc<AtomicUsize>,
}

impl ByteGuard {
    fn add_amount(&mut self, amount: usize) -> bool {
        let Some(next) = self.amount.checked_add(amount) else {
            return false;
        };
        self.amount = next;
        true
    }
}

impl Drop for ByteGuard {
    fn drop(&mut self) {
        self.in_flight_bytes
            .fetch_sub(self.amount, Ordering::Relaxed);
    }
}

impl AdmissionController {
    pub fn new(max_connections: usize, max_in_flight_bytes: usize) -> Self {
        Self {
            max_connections,
            max_in_flight_bytes,
            active_connections: Arc::new(AtomicUsize::new(0)),
            in_flight_bytes: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn acquire_connection(&self) -> Option<ConnectionGuard> {
        let mut current = self.active_connections.load(Ordering::Relaxed);
        loop {
            if current >= self.max_connections {
                return None;
            }
            match self.active_connections.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(ConnectionGuard {
                        active_connections: self.active_connections.clone(),
                    });
                }
                Err(actual) => current = actual,
            }
        }
    }

    pub fn acquire_bytes(&self, amount: usize) -> Option<ByteGuard> {
        self.reserve_bytes(amount).map(|_| ByteGuard {
            amount,
            in_flight_bytes: self.in_flight_bytes.clone(),
        })
    }

    pub fn acquire_more_bytes(&self, guard: &mut ByteGuard, amount: usize) -> bool {
        if self.reserve_bytes(amount).is_none() {
            return false;
        }
        if guard.add_amount(amount) {
            true
        } else {
            self.in_flight_bytes.fetch_sub(amount, Ordering::Relaxed);
            false
        }
    }

    fn reserve_bytes(&self, amount: usize) -> Option<()> {
        let mut current = self.in_flight_bytes.load(Ordering::Relaxed);
        loop {
            let next = current.checked_add(amount)?;
            if next > self.max_in_flight_bytes {
                return None;
            }
            match self.in_flight_bytes.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(()),
                Err(actual) => current = actual,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admission_limits() {
        let controller = AdmissionController::new(2, 1024);
        let g1 = controller.acquire_connection().unwrap();
        let _g2 = controller.acquire_connection().unwrap();
        assert!(controller.acquire_connection().is_none());
        drop(g1);
        let g3 = controller.acquire_connection();
        assert!(g3.is_some());
    }

    #[test]
    fn test_byte_guard() {
        let controller = AdmissionController::new(10, 100);
        let b1 = controller.acquire_bytes(60).unwrap();
        assert!(controller.acquire_bytes(50).is_none());
        drop(b1);
        assert!(controller.acquire_bytes(50).is_some());
    }

    #[test]
    fn concurrent_connections_never_exceed_limit() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        const CONTENDERS: usize = 32;
        let controller = Arc::new(AdmissionController::new(1, 1024));
        let start = Arc::new(Barrier::new(CONTENDERS));
        let acquired = Arc::new(Barrier::new(CONTENDERS));
        let handles: Vec<_> = (0..CONTENDERS)
            .map(|_| {
                let controller = controller.clone();
                let start = start.clone();
                let acquired = acquired.clone();
                thread::spawn(move || {
                    start.wait();
                    let guard = controller.acquire_connection();
                    let succeeded = guard.is_some();
                    acquired.wait();
                    (succeeded, guard)
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            results.iter().filter(|(succeeded, _)| *succeeded).count(),
            1
        );
        drop(results);
        assert!(controller.acquire_connection().is_some());
    }

    #[test]
    fn concurrent_byte_acquisitions_never_exceed_limit() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        const CONTENDERS: usize = 32;
        let controller = Arc::new(AdmissionController::new(10, 100));
        let start = Arc::new(Barrier::new(CONTENDERS));
        let acquired = Arc::new(Barrier::new(CONTENDERS));
        let handles: Vec<_> = (0..CONTENDERS)
            .map(|_| {
                let controller = controller.clone();
                let start = start.clone();
                let acquired = acquired.clone();
                thread::spawn(move || {
                    start.wait();
                    let guard = controller.acquire_bytes(60);
                    let succeeded = guard.is_some();
                    acquired.wait();
                    (succeeded, guard)
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            results.iter().filter(|(succeeded, _)| *succeeded).count(),
            1
        );
        drop(results);
        assert!(controller.acquire_bytes(100).is_some());
    }
}
