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
        let current = self.active_connections.load(Ordering::Relaxed);
        if current >= self.max_connections {
            return None;
        }
        self.active_connections.fetch_add(1, Ordering::Relaxed);
        Some(ConnectionGuard {
            active_connections: self.active_connections.clone(),
        })
    }

    pub fn acquire_bytes(&self, amount: usize) -> Option<ByteGuard> {
        let current = self.in_flight_bytes.load(Ordering::Relaxed);
        if current.saturating_add(amount) > self.max_in_flight_bytes {
            return None;
        }
        self.in_flight_bytes.fetch_add(amount, Ordering::Relaxed);
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
}
