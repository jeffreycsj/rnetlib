//! Independent payload and ordering checks for the long-running loopback probe.

use rnet_core::Transport;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProbeDisposition {
    New,
    Reordered,
    Duplicate,
    TooOld,
}

#[derive(Default)]
struct SequenceWindow {
    highest: Option<u64>,
    seen: u64,
}

/// Counts distinct valid messages; reliable transports must remain strictly ordered.
pub(crate) struct ProbeReceiver {
    transport: Transport,
    payload_bytes: usize,
    windows: Vec<SequenceWindow>,
    pub(crate) received: u64,
    pub(crate) duplicates: u64,
    pub(crate) reordered: u64,
    pub(crate) too_old: u64,
}

impl ProbeReceiver {
    pub(crate) fn new(transport: Transport, clients: usize, payload_bytes: usize) -> Self {
        Self {
            transport,
            payload_bytes,
            windows: (0..clients).map(|_| SequenceWindow::default()).collect(),
            received: 0,
            duplicates: 0,
            reordered: 0,
            too_old: 0,
        }
    }

    pub(crate) fn observe(
        &mut self,
        payload: &[u8],
        sent_counts: &[u64],
    ) -> Result<ProbeDisposition, &'static str> {
        if payload.len() != self.payload_bytes || payload.len() < 12 {
            return Err("probe payload length changed");
        }
        if payload[12..].iter().any(|byte| *byte != 0x5a) {
            return Err("probe payload was corrupted");
        }
        let client =
            u32::from_be_bytes(payload[..4].try_into().expect("validated client ID")) as usize;
        let sequence = u64::from_be_bytes(payload[4..12].try_into().expect("validated sequence"));
        let sent = *sent_counts
            .get(client)
            .ok_or("probe has unknown client ID")?;
        let window = self
            .windows
            .get_mut(client)
            .ok_or("probe has unknown client ID")?;
        if sequence >= sent {
            return Err("probe contains a sequence that was never sent");
        }
        if self.transport != Transport::Udp {
            let expected = window
                .highest
                .map_or(0, |highest| highest.saturating_add(1));
            if sequence != expected {
                return Err("reliable transport lost, reordered, or duplicated a probe");
            }
            window.highest = Some(sequence);
            self.received = self.received.saturating_add(1);
            return Ok(ProbeDisposition::New);
        }
        let Some(highest) = window.highest else {
            window.highest = Some(sequence);
            window.seen = 1;
            self.received = self.received.saturating_add(1);
            return Ok(ProbeDisposition::New);
        };
        if sequence > highest {
            let forward = sequence - highest;
            window.highest = Some(sequence);
            window.seen = if forward >= 64 {
                1
            } else {
                (window.seen << forward) | 1
            };
            self.received = self.received.saturating_add(1);
            return Ok(ProbeDisposition::New);
        }
        let behind = highest - sequence;
        if behind >= 64 {
            self.too_old = self.too_old.saturating_add(1);
            return Ok(ProbeDisposition::TooOld);
        }
        let bit = 1_u64 << behind;
        if window.seen & bit != 0 {
            self.duplicates = self.duplicates.saturating_add(1);
            return Ok(ProbeDisposition::Duplicate);
        }
        window.seen |= bit;
        self.received = self.received.saturating_add(1);
        self.reordered = self.reordered.saturating_add(1);
        Ok(ProbeDisposition::Reordered)
    }
}
