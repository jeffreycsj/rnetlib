use crate::metrics::AdmissionRejectReason;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum IpKey {
    V4([u8; 4]),
    V6([u8; 16]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionReject {
    Active,
    Rate,
    Table,
}

impl AdmissionReject {
    pub(crate) const fn metric_reason(self) -> AdmissionRejectReason {
        match self {
            Self::Active => AdmissionRejectReason::IpActiveLimit,
            Self::Rate => AdmissionRejectReason::IpRateLimit,
            Self::Table => AdmissionRejectReason::IpTableLimit,
        }
    }
}

#[derive(Debug)]
struct Entry {
    active: usize,
    tokens: f64,
    updated_at: Instant,
}

#[derive(Debug)]
struct AdmissionInner {
    entries: Mutex<std::collections::HashMap<IpKey, Entry>>,
    max_active: usize,
    refill_per_second: u32,
    burst: u32,
    max_entries: usize,
    ipv6_prefix_bits: u8,
}

#[derive(Clone, Debug)]
pub(crate) struct AdmissionController(Arc<AdmissionInner>);

#[derive(Debug)]
pub(crate) struct AdmissionPermit {
    owner: Weak<AdmissionInner>,
    key: IpKey,
}

impl AdmissionController {
    pub(crate) fn new(
        max_active: usize,
        refill_per_second: u32,
        burst: u32,
        max_entries: usize,
        ipv6_prefix_bits: u8,
    ) -> Self {
        Self(Arc::new(AdmissionInner {
            entries: Mutex::new(std::collections::HashMap::new()),
            max_active,
            refill_per_second,
            burst,
            max_entries,
            ipv6_prefix_bits,
        }))
    }

    pub(crate) fn try_acquire(&self, ip: IpAddr) -> Result<AdmissionPermit, AdmissionReject> {
        self.try_acquire_at(ip, Instant::now())
    }

    fn try_acquire_at(&self, ip: IpAddr, now: Instant) -> Result<AdmissionPermit, AdmissionReject> {
        let key = ip_key(ip, self.0.ipv6_prefix_bits);
        let mut entries = self.0.entries.lock().expect("IP admission table poisoned");
        if !entries.contains_key(&key) && entries.len() >= self.0.max_entries {
            entries.retain(|_, entry| entry.active != 0 || entry.tokens < f64::from(self.0.burst));
            if entries.len() >= self.0.max_entries {
                return Err(AdmissionReject::Table);
            }
        }
        let entry = entries.entry(key).or_insert(Entry {
            active: 0,
            tokens: f64::from(self.0.burst),
            updated_at: now,
        });
        let elapsed = now
            .saturating_duration_since(entry.updated_at)
            .as_secs_f64();
        entry.tokens = (entry.tokens + elapsed * f64::from(self.0.refill_per_second))
            .min(f64::from(self.0.burst));
        entry.updated_at = now;
        if entry.active >= self.0.max_active {
            return Err(AdmissionReject::Active);
        }
        if entry.tokens < 1.0 {
            return Err(AdmissionReject::Rate);
        }
        entry.tokens -= 1.0;
        entry.active += 1;
        Ok(AdmissionPermit {
            owner: Arc::downgrade(&self.0),
            key,
        })
    }
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let mut entries = owner.entries.lock().expect("IP admission table poisoned");
        if let Some(entry) = entries.get_mut(&self.key) {
            entry.active = entry.active.saturating_sub(1);
        }
    }
}

fn ip_key(ip: IpAddr, ipv6_prefix_bits: u8) -> IpKey {
    match ip {
        IpAddr::V4(ip) => IpKey::V4(ip.octets()),
        IpAddr::V6(ip) => {
            let mut octets = ip.octets();
            let prefix = ipv6_prefix_bits.min(128);
            let whole = usize::from(prefix / 8);
            let remainder = prefix % 8;
            if remainder != 0 && whole < octets.len() {
                octets[whole] &= u8::MAX << (8 - remainder);
            }
            let clear_from = whole + usize::from(remainder != 0);
            octets[clear_from..].fill(0);
            IpKey::V6(octets)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AdmissionController, AdmissionReject};
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::{Duration, Instant};

    #[test]
    fn admission_enforces_active_and_rate_limits_and_recovers() {
        let controller = AdmissionController::new(1, 1, 2, 16, 64);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let now = Instant::now();

        let first = controller.try_acquire_at(ip, now).unwrap();
        assert_eq!(
            controller.try_acquire_at(ip, now).unwrap_err(),
            AdmissionReject::Active
        );
        drop(first);
        let second = controller.try_acquire_at(ip, now).unwrap();
        drop(second);
        assert_eq!(
            controller.try_acquire_at(ip, now).unwrap_err(),
            AdmissionReject::Rate
        );
        assert!(controller
            .try_acquire_at(ip, now + Duration::from_secs(1))
            .is_ok());
    }
}
