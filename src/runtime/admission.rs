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
        self.active_connections
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(1)
                    .filter(|next| *next <= self.max_connections)
            })
            .ok()?;
        Some(ConnectionGuard {
            active_connections: self.active_connections.clone(),
        })
    }

    pub fn acquire_bytes(&self, amount: usize) -> Option<ByteGuard> {
        self.in_flight_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(amount)
                    .filter(|next| *next <= self.max_in_flight_bytes)
            })
            .ok()?;
        Some(ByteGuard {
            amount,
            in_flight_bytes: self.in_flight_bytes.clone(),
        })
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
