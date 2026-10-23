//! Benchmark payload format only; this is not an SDK/network wire version.

use super::{credentials::Identity, Result};
use rnet_core::Transport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Data,
    Finish,
    Ack,
}

pub struct Packet {
    pub kind: Kind,
    pub identity: Identity,
    /// Data sequence, or final admitted data count for Finish/Ack.
    pub sequence: u64,
}

impl Packet {
    pub fn encode(&self, payload: usize) -> Vec<u8> {
        let mut bytes = vec![0x5a; if self.kind == Kind::Data { payload } else { 32 }];
        bytes[..4].copy_from_slice(match self.kind {
            Kind::Data => b"RNPD",
            Kind::Finish => b"RNPF",
            Kind::Ack => b"RNPA",
        });
        bytes[4..24].copy_from_slice(&self.identity.bytes());
        bytes[24..32].copy_from_slice(&self.sequence.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8], payload: usize) -> Result<Self> {
        if bytes.len() < 32 {
            return Err("probe payload too short".into());
        }
        let kind = match &bytes[..4] {
            b"RNPD" => Kind::Data,
            b"RNPF" => Kind::Finish,
            b"RNPA" => Kind::Ack,
            _ => return Err("invalid probe payload tag".into()),
        };
        let length = if kind == Kind::Data { payload } else { 32 };
        if bytes.len() != length || bytes[32..].iter().any(|byte| *byte != 0x5a) {
            return Err("probe payload length or content mismatch".into());
        }
        Ok(Self {
            kind,
            identity: Identity::decode(&bytes[4..24])?,
            sequence: u64::from_be_bytes(bytes[24..32].try_into()?),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    New,
    Reordered,
    Duplicate,
    TooOld,
}

/// A bounded 64-sequence receive history. Old UDP packets cannot be counted as unique.
#[derive(Default)]
pub struct Window {
    pub next: u64,
    seen: u64,
}

impl Window {
    pub fn observe(&mut self, sequence: u64, transport: Transport) -> Result<Disposition> {
        if sequence == u64::MAX {
            return Err("probe sequence overflow".into());
        }
        if transport != Transport::Udp {
            if sequence != self.next {
                return Err("reliable probe sequence gap, duplicate or reordering".into());
            }
            self.next += 1;
            return Ok(Disposition::New);
        }
        if sequence >= self.next {
            let advance = sequence + 1 - self.next;
            self.seen = if advance >= 64 {
                1
            } else {
                (self.seen << advance) | 1
            };
            self.next = sequence + 1;
            return Ok(Disposition::New);
        }
        let behind = self.next - 1 - sequence;
        if behind >= 64 {
            return Ok(Disposition::TooOld);
        }
        let bit = 1 << behind;
        if self.seen & bit != 0 {
            return Ok(Disposition::Duplicate);
        }
        self.seen |= bit;
        Ok(Disposition::Reordered)
    }
}
