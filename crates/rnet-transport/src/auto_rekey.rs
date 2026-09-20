use std::time::{Duration, Instant};

/// Tracks server-side automatic key rotation without coupling policy to a transport driver.
pub(crate) struct AutoRekey {
    after: Option<Duration>,
    after_bytes: Option<u64>,
    last_started: Instant,
    encrypted_bytes: u64,
}

impl AutoRekey {
    pub(crate) fn new(after: Option<Duration>, after_bytes: Option<u64>, now: Instant) -> Self {
        Self {
            after,
            after_bytes,
            last_started: now,
            encrypted_bytes: 0,
        }
    }

    pub(crate) fn record_encrypted_bytes(&mut self, bytes: usize) {
        self.encrypted_bytes = self.encrypted_bytes.saturating_add(bytes as u64);
    }

    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.after
            .is_some_and(|threshold| now.duration_since(self.last_started) >= threshold)
            || self
                .after_bytes
                .is_some_and(|threshold| self.encrypted_bytes >= threshold)
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.after
            .and_then(|threshold| self.last_started.checked_add(threshold))
    }

    pub(crate) fn mark_started(&mut self, now: Instant) {
        self.last_started = now;
        self.encrypted_bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::AutoRekey;
    use std::time::{Duration, Instant};

    #[test]
    fn rekey_is_due_on_either_time_or_encrypted_byte_threshold() {
        let started = Instant::now();
        let mut by_bytes = AutoRekey::new(Some(Duration::from_secs(60)), Some(10), started);
        by_bytes.record_encrypted_bytes(9);
        assert!(!by_bytes.is_due(started + Duration::from_secs(1)));
        by_bytes.record_encrypted_bytes(1);
        assert!(by_bytes.is_due(started + Duration::from_secs(1)));
        by_bytes.mark_started(started + Duration::from_secs(1));
        assert!(!by_bytes.is_due(started + Duration::from_secs(2)));

        let by_time = AutoRekey::new(Some(Duration::from_secs(5)), None, started);
        assert!(!by_time.is_due(started + Duration::from_secs(4)));
        assert!(by_time.is_due(started + Duration::from_secs(5)));
    }
}
