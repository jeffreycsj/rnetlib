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
    let removed = guard.remove(handle & !GAME_HANDLE_TAG);
    // Dropping the runtime joins its logger. User callbacks may query another runtime (or the
    // just-removed handle), so neither the join nor user code may run under the registry mutex.
    drop(guard);
    drop(removed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{destroy, lease, register, table, GAME_HANDLE_TAG};
    use rnet_game::{BoundedLogger, GameRuntime, GameRuntimeConfig, LoggerConfig};
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn destroy_drops_the_logger_outside_the_global_registry_lock() {
        let (started_tx, started_rx) = mpsc::channel();
        let (query_tx, query_rx) = mpsc::channel();
        let other = register(GameRuntime::new(GameRuntimeConfig::production()).unwrap());
        let logger = BoundedLogger::new(LoggerConfig::default(), move |_| {
            let _ = started_tx.send(());
            if query_rx.recv().is_ok() {
                assert!(lease(other).is_ok());
            }
        })
        .unwrap();
        let handle = register(
            GameRuntime::new(GameRuntimeConfig::production())
                .unwrap()
                .with_logger(logger),
        );
        {
            let entry = lease(handle).unwrap();
            entry.runtime.poll(1, Duration::ZERO);
        }
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        {
            let entry = lease(handle).unwrap();
            entry.runtime.stop(Duration::ZERO).unwrap();
            entry.stopped.store(true, Ordering::Release);
        }
        let worker = std::thread::spawn(move || destroy(handle));
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut removed_without_lock = false;
        while Instant::now() < deadline {
            if let Ok(guard) = table().try_lock() {
                if guard.get(handle & !GAME_HANDLE_TAG).is_none() {
                    removed_without_lock = true;
                    break;
                }
            }
            std::thread::yield_now();
        }
        assert!(
            removed_without_lock,
            "registry mutex held while joining a user callback"
        );
        query_tx.send(()).unwrap();
        worker.join().unwrap().unwrap();
        {
            let entry = lease(other).unwrap();
            entry.runtime.stop(Duration::ZERO).unwrap();
            entry.stopped.store(true, Ordering::Release);
        }
        destroy(other).unwrap();
    }
}
