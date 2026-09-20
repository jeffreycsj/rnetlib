//! Game runtime and event-buffer lifetimes are independent from the legacy transport ABI.

use crate::registry::invalid_state;
use rnet_core::{BufferView, ErrorCode, HandleTable, Result, RnetError};
use rnet_game::GameRuntime;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use zeroize::Zeroizing;

const GAME_HANDLE_TAG: u64 = 1 << 63;

pub(crate) struct GameEntry {
    pub(crate) runtime: GameRuntime,
    pub(crate) buffers: GameBuffers,
    pub(crate) stopped: AtomicBool,
    pub(crate) active_calls: AtomicUsize,
}

pub(crate) struct GameLease(Arc<GameEntry>);

impl Deref for GameLease {
    type Target = GameEntry;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for GameLease {
    fn drop(&mut self) {
        self.active_calls.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Default)]
pub(crate) struct GameBuffers(Mutex<HandleTable<Zeroizing<Vec<u8>>>>);

impl GameBuffers {
    pub(crate) fn insert(&self, bytes: Vec<u8>) -> Option<BufferView> {
        if bytes.is_empty() {
            return None;
        }
        let bytes = Zeroizing::new(bytes);
        let ptr = bytes.as_ptr();
        let len = bytes.len();
        let token = self
            .0
            .lock()
            .expect("game buffer table poisoned")
            .insert(bytes);
        Some(BufferView { token, ptr, len })
    }

    pub(crate) fn release(&self, token: u64) -> Result<()> {
        self.0
            .lock()
            .expect("game buffer table poisoned")
            .remove(token)
            .map(drop)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid game buffer token"))
    }

    pub(crate) fn outstanding(&self) -> usize {
        self.0.lock().expect("game buffer table poisoned").len()
    }
}

static GAME_RUNTIMES: OnceLock<Mutex<HandleTable<Arc<GameEntry>>>> = OnceLock::new();

fn table() -> &'static Mutex<HandleTable<Arc<GameEntry>>> {
    GAME_RUNTIMES.get_or_init(|| Mutex::new(HandleTable::new()))
}

pub(crate) fn register(runtime: GameRuntime) -> u64 {
    let entry = Arc::new(GameEntry {
        runtime,
        buffers: GameBuffers::default(),
        stopped: AtomicBool::new(false),
        active_calls: AtomicUsize::new(0),
    });
    table()
        .lock()
        .expect("game runtime table poisoned")
        .insert(entry)
        | GAME_HANDLE_TAG
}

pub(crate) fn lease(handle: u64) -> Result<GameLease> {
    if handle & GAME_HANDLE_TAG == 0 {
        return Err(RnetError::new(
            ErrorCode::InvalidHandle,
            "not a game runtime",
        ));
    }
    let guard = table().lock().expect("game runtime table poisoned");
    let entry = guard
        .get(handle & !GAME_HANDLE_TAG)
        .cloned()
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid game runtime"))?;
    entry.active_calls.fetch_add(1, Ordering::AcqRel);
    Ok(GameLease(entry))
}

pub(crate) fn destroy(handle: u64) -> Result<()> {
    if handle & GAME_HANDLE_TAG == 0 {
        return Err(RnetError::new(
            ErrorCode::InvalidHandle,
            "not a game runtime",
        ));
    }
    let mut guard = table().lock().expect("game runtime table poisoned");
    let entry = guard
        .get(handle & !GAME_HANDLE_TAG)
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid game runtime"))?;
    if !entry.stopped.load(Ordering::Acquire) {
        return invalid_state("game runtime must be stopped before destroy");
    }
    if entry.buffers.outstanding() != 0 {
        return invalid_state("release all game event buffers before destroy");
    }
    if entry.active_calls.load(Ordering::Acquire) != 0 {
        return Err(RnetError::new(
            ErrorCode::WouldBlock,
            "game runtime has active calls",
        ));
    }
    guard.remove(handle & !GAME_HANDLE_TAG);
    Ok(())
}
