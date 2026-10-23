//! Bounded application echo retries: FIFO per session, round-robin across sessions.

use super::{wire::Kind, Result};
use bytes::Bytes;
use rnet_core::{ErrorCode, Handle};
use std::collections::{HashMap, VecDeque};

struct Pending {
    payload: Bytes,
    kind: Kind,
}

pub struct EchoQueue {
    queues: HashMap<Handle, VecDeque<Pending>>,
    ready: VecDeque<Handle>,
    pub messages: usize,
    pub bytes: usize,
    max_messages: usize,
    max_bytes: usize,
}

#[derive(Default)]
pub struct Forwarded {
    pub data: u64,
    pub blocked: u64,
    pub error: Option<rnet_core::RnetError>,
}

impl EchoQueue {
    pub fn new(max_messages: usize, max_bytes: usize) -> Self {
        Self {
            queues: HashMap::new(),
            ready: VecDeque::new(),
            messages: 0,
            bytes: 0,
            max_messages,
            max_bytes,
        }
    }

    pub fn push(&mut self, session: Handle, payload: Bytes, kind: Kind) -> Result<()> {
        if self.messages >= self.max_messages
            || payload.len() > self.max_bytes.saturating_sub(self.bytes)
        {
            return Err("echo_retry_budget_exhausted".into());
        }
        let queue = self.queues.entry(session).or_insert_with(|| {
            self.ready.push_back(session);
            VecDeque::new()
        });
        self.messages += 1;
        self.bytes += payload.len();
        queue.push_back(Pending { payload, kind });
        Ok(())
    }

    /// A completed peer may close after one ACK while duplicate UDP ACK retries remain.
    pub fn remove(&mut self, session: Handle) -> bool {
        let Some(queue) = self.queues.remove(&session) else {
            return false;
        };
        self.ready.retain(|value| *value != session);
        let data = queue.iter().any(|item| item.kind == Kind::Data);
        self.messages -= queue.len();
        self.bytes -= queue.iter().map(|item| item.payload.len()).sum::<usize>();
        data
    }

    /// Partial admission remains reportable even if a later send fails permanently.
    pub fn flush(
        &mut self,
        mut send: impl FnMut(Handle, &[u8]) -> rnet_core::Result<()>,
    ) -> Forwarded {
        let mut result = Forwarded::default();
        let mut consecutive_blocks = 0;
        for _ in 0..256 {
            let Some(session) = self.ready.pop_front() else {
                break;
            };
            let queue = self
                .queues
                .get_mut(&session)
                .expect("ready session owns a queue");
            let item = queue.front().expect("ready queues are nonempty");
            match send(session, &item.payload) {
                Ok(()) => {
                    consecutive_blocks = 0;
                    self.messages -= 1;
                    self.bytes -= item.payload.len();
                    result.data += u64::from(item.kind == Kind::Data);
                    queue.pop_front();
                }
                Err(error) if error.code() == ErrorCode::WouldBlock => {
                    result.blocked += 1;
                    consecutive_blocks += 1;
                }
                Err(error) => {
                    self.ready.push_front(session);
                    result.error = Some(error);
                    break;
                }
            }
            if queue.is_empty() {
                self.queues.remove(&session);
            } else {
                self.ready.push_back(session);
            }
            if consecutive_blocks > 0 && consecutive_blocks >= self.ready.len() {
                break;
            }
        }
        result
    }
}
