//! Single-runtime, single-use resume tickets. The registry is protected by the game runtime's
//! mutex; it deliberately does not imply recovery across a process restart or another server.

use crate::config::GameProtocol;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use rnet_core::{ErrorCode, Handle, Result, RnetError};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};
use zeroize::Zeroize;

const NONCE_LEN: usize = 16;
pub(crate) const TICKET_LEN: usize = NONCE_LEN + 32;
const SIGNING_CONTEXT: &[u8] = b"rnet-game-resume-v1";
const MAX_IDENTITY_LEN: usize = 64;
const MAX_TTL: Duration = Duration::from_secs(300);

pub(crate) struct ResumeClaim {
    pub(crate) old_session: Handle,
    pub(crate) identity: Vec<u8>,
}

impl Drop for ResumeClaim {
    fn drop(&mut self) {
        self.identity.zeroize();
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ResumeScope {
    pub(crate) endpoint: Handle,
    pub(crate) session: Handle,
    pub(crate) peer_key: [u8; 32],
    pub(crate) protocol: GameProtocol,
}

struct TicketEntry {
    endpoint: Handle,
    old_session: Handle,
    peer_key: [u8; 32],
    protocol_id: u64,
    protocol_version: u32,
    identity: Vec<u8>,
    expires: Instant,
}

impl Drop for TicketEntry {
    fn drop(&mut self) {
        self.identity.zeroize();
    }
}

pub(crate) struct ResumeRegistry {
    key: hmac::Key,
    max_entries: usize,
    entries: HashMap<[u8; NONCE_LEN], TicketEntry>,
    by_session: HashMap<Handle, [u8; NONCE_LEN]>,
    expiries: BTreeMap<Instant, HashSet<[u8; NONCE_LEN]>>,
}

impl ResumeRegistry {
    pub(crate) fn validate_ttl(ttl: Duration) -> Result<()> {
        if ttl.is_zero() || ttl > MAX_TTL {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "resume ticket lifetime must be between 1 ns and 5 minutes",
            ));
        }
        Ok(())
    }

    pub(crate) fn new(max_entries: usize) -> Result<Self> {
        if !(1..=65_536).contains(&max_entries) {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "resume ticket capacity is outside the supported range",
            ));
        }
        let mut secret = [0_u8; 32];
        SystemRandom::new()
            .fill(&mut secret)
            .map_err(|_| RnetError::new(ErrorCode::CryptoError, "resume ticket RNG failed"))?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, &secret);
        secret.zeroize();
        Ok(Self {
            key,
            max_entries,
            entries: HashMap::new(),
            by_session: HashMap::new(),
            expiries: BTreeMap::new(),
        })
    }

    /// Reissuing for one old session invalidates its previous ticket atomically.
    pub(crate) fn issue(
        &mut self,
        scope: ResumeScope,
        identity: &[u8],
        now: Instant,
        ttl: Duration,
    ) -> Result<Vec<u8>> {
        if scope.endpoint == 0
            || scope.session == 0
            || !scope.protocol.is_valid()
            || identity.is_empty()
            || identity.len() > MAX_IDENTITY_LEN
        {
            return Err(RnetError::new(
                ErrorCode::InvalidArgument,
                "invalid resume ticket scope or lifetime",
            ));
        }
        Self::validate_ttl(ttl)?;
        let expires = now.checked_add(ttl).ok_or_else(|| {
            RnetError::new(ErrorCode::InvalidArgument, "resume deadline overflows")
        })?;
        self.prune(now);
        if self.entries.len() >= self.max_entries && !self.by_session.contains_key(&scope.session) {
            return Err(RnetError::new(
                ErrorCode::WouldBlock,
                "resume ticket capacity is full",
            ));
        }
        let nonce = self.new_nonce()?;
        let mut ticket = vec![0; TICKET_LEN];
        ticket[..NONCE_LEN].copy_from_slice(&nonce);
        ticket[NONCE_LEN..].copy_from_slice(self.sign(nonce).as_ref());
        self.revoke_session(scope.session);
        self.entries.insert(
            nonce,
            TicketEntry {
                endpoint: scope.endpoint,
                old_session: scope.session,
                peer_key: scope.peer_key,
                protocol_id: scope.protocol.protocol_id,
                protocol_version: scope.protocol.version,
                identity: identity.to_vec(),
                expires,
            },
        );
        self.by_session.insert(scope.session, nonce);
        self.expiries.entry(expires).or_default().insert(nonce);
        Ok(ticket)
    }

    /// An invalid peer/protocol cannot burn another client's ticket. A valid claim is consumed
    /// before application authorization, so a rejected authorization cannot replay it.
    pub(crate) fn consume(
        &mut self,
        ticket: &[u8],
        endpoint: Handle,
        peer_key: [u8; 32],
        protocol: GameProtocol,
        now: Instant,
    ) -> Result<ResumeClaim> {
        self.prune(now);
        if ticket.len() != TICKET_LEN {
            return Err(invalid_ticket());
        }
        let nonce: [u8; NONCE_LEN] = ticket[..NONCE_LEN].try_into().expect("ticket length");
        hmac::verify(&self.key, &signing_input(nonce), &ticket[NONCE_LEN..])
            .map_err(|_| invalid_ticket())?;
        let Some(entry) = self.entries.get(&nonce) else {
            return Err(invalid_ticket());
        };
        if entry.endpoint != endpoint
            || entry.peer_key != peer_key
            || entry.protocol_id != protocol.protocol_id
            || entry.protocol_version != protocol.version
            || entry.expires <= now
        {
            return Err(invalid_ticket());
        }
        let mut entry = self.remove(nonce).expect("verified ticket exists");
        Ok(ResumeClaim {
            old_session: entry.old_session,
            identity: std::mem::take(&mut entry.identity),
        })
    }

    /// Reads the version pinned by an authenticated, unexpired ticket without consuming it.
    /// A subsequent `consume` under the same registry lock performs the final one-use claim.
    pub(crate) fn pinned_version(
        &mut self,
        ticket: &[u8],
        endpoint: Handle,
        peer_key: [u8; 32],
        protocol_id: u64,
        now: Instant,
    ) -> Result<u32> {
        self.prune(now);
        if ticket.len() != TICKET_LEN {
            return Err(invalid_ticket());
        }
        let nonce: [u8; NONCE_LEN] = ticket[..NONCE_LEN].try_into().expect("ticket length");
        hmac::verify(&self.key, &signing_input(nonce), &ticket[NONCE_LEN..])
            .map_err(|_| invalid_ticket())?;
        let entry = self.entries.get(&nonce).ok_or_else(invalid_ticket)?;
        if entry.endpoint != endpoint
            || entry.peer_key != peer_key
            || entry.protocol_id != protocol_id
            || entry.expires <= now
        {
            return Err(invalid_ticket());
        }
        Ok(entry.protocol_version)
    }

    /// Explicit kicks revoke recovery. A network disconnect leaves the ticket alive until expiry.
    pub(crate) fn revoke_session(&mut self, session: Handle) {
        if let Some(nonce) = self.by_session.get(&session).copied() {
            self.remove(nonce);
        }
    }

    pub(crate) fn revoke_endpoint(&mut self, endpoint: Handle) {
        let nonces: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(nonce, entry)| (entry.endpoint == endpoint).then_some(*nonce))
            .collect();
        for nonce in nonces {
            self.remove(nonce);
        }
    }

    pub(crate) fn revoke_ticket(&mut self, ticket: &[u8]) {
        if ticket.len() != TICKET_LEN {
            return;
        }
        let nonce: [u8; NONCE_LEN] = ticket[..NONCE_LEN].try_into().expect("ticket length");
        if self.entries.contains_key(&nonce) {
            self.remove(nonce);
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.by_session.clear();
        self.expiries.clear();
    }

    pub(crate) fn outstanding(&self) -> usize {
        self.entries.len()
    }

    fn new_nonce(&self) -> Result<[u8; NONCE_LEN]> {
        let random = SystemRandom::new();
        for _ in 0..4 {
            let mut nonce = [0; NONCE_LEN];
            random
                .fill(&mut nonce)
                .map_err(|_| RnetError::new(ErrorCode::CryptoError, "resume ticket RNG failed"))?;
            if !self.entries.contains_key(&nonce) {
                return Ok(nonce);
            }
        }
        Err(RnetError::new(
            ErrorCode::CryptoError,
            "resume ticket nonce collision",
        ))
    }

    fn sign(&self, nonce: [u8; NONCE_LEN]) -> hmac::Tag {
        hmac::sign(&self.key, &signing_input(nonce))
    }

    fn remove(&mut self, nonce: [u8; NONCE_LEN]) -> Option<TicketEntry> {
        let entry = self.entries.remove(&nonce)?;
        self.by_session.remove(&entry.old_session);
        let empty = self.expiries.get_mut(&entry.expires).is_some_and(|set| {
            set.remove(&nonce);
            set.is_empty()
        });
        if empty {
            self.expiries.remove(&entry.expires);
        }
        Some(entry)
    }

    fn prune(&mut self, now: Instant) {
        while self
            .expiries
            .first_key_value()
            .is_some_and(|(at, _)| *at <= now)
        {
            let (_, nonces) = self.expiries.pop_first().expect("expired bucket exists");
            for nonce in nonces {
                if let Some(entry) = self.entries.remove(&nonce) {
                    self.by_session.remove(&entry.old_session);
                }
            }
        }
    }
}

fn signing_input(nonce: [u8; NONCE_LEN]) -> [u8; SIGNING_CONTEXT.len() + NONCE_LEN] {
    let mut input = [0; SIGNING_CONTEXT.len() + NONCE_LEN];
    input[..SIGNING_CONTEXT.len()].copy_from_slice(SIGNING_CONTEXT);
    input[SIGNING_CONTEXT.len()..].copy_from_slice(&nonce);
    input
}

fn invalid_ticket() -> RnetError {
    RnetError::new(ErrorCode::ProtocolError, "invalid resume ticket")
}
