//! buffer contracts shared by runtime and foreign-language boundaries.

use crate::{ErrorCode, Handle, HandleTable, Result, RnetError};
use std::sync::Mutex;

/// Non-owning exported address plus owner-local release token. Copying this value does not
/// extend storage lifetime or create another release obligation. Keep the owner alive.
#[derive(Clone, Copy, Debug)]
pub struct BufferView {
    /// Release exactly once through the originating BufferStore, including for an empty buffer.
    pub token: Handle,
    /// Read-only address valid until token release or store drop; null exactly when len is zero.
    /// Pointer reads must not race release; this view does not hold a lifetime guard.
    pub ptr: *const u8,
    /// Readable byte length, excluding any terminator; zero permits a null pointer.
    pub len: usize,
}

/// Mutex-protected storage for exported byte buffers. Each instance owns its token namespace.
/// There is no built-in byte/count budget: the embedding API must limit outstanding exports.
/// Dropping the store invalidates all pointers; callers must coordinate external readers.
#[derive(Debug, Default)]
pub struct BufferStore {
    buffers: Mutex<HandleTable<Box<[u8]>>>,
}

impl BufferStore {
    /// Creates an empty store; exported memory is allocated on insert.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes ownership of bytes; the address stays valid until token release or store drop.
    ///
    /// Callers must serialize release with all reads through `ptr`; releasing a token while
    /// another thread reads its pointer violates the external API contract.
    /// An empty input still allocates a release token but returns a null address and zero length.
    pub fn insert(&self, bytes: Vec<u8>) -> BufferView {
        let boxed = bytes.into_boxed_slice();
        let ptr = if boxed.is_empty() {
            std::ptr::null()
        } else {
            boxed.as_ptr()
        };
        let len = boxed.len();
        let token = self
            .buffers
            .lock()
            .expect("buffer store poisoned")
            .insert(boxed);
        BufferView { token, ptr, len }
    }

    /// Frees one token owned by this store; unknown/already-released tokens return `InvalidHandle`.
    /// Call only after all external reads finish. Tokens from other stores are forbidden,
    /// not guaranteed to be distinguishable. Memory is freed, not securely zeroized.
    pub fn release(&self, token: Handle) -> Result<()> {
        self.buffers
            .lock()
            .expect("buffer store poisoned")
            .remove(token)
            .map(drop)
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid buffer token"))
    }

    /// Copies live bytes under the storage lock; the independent result survives later release.
    /// An invalid token returns `InvalidHandle`; original ownership/release duty is unchanged.
    pub fn copy(&self, token: Handle) -> Result<Vec<u8>> {
        self.buffers
            .lock()
            .expect("buffer store poisoned")
            .get(token)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid buffer token"))
    }

    /// Counts unreleased buffers, including empty ones; this is a gauge, not retained bytes.
    pub fn outstanding(&self) -> usize {
        self.buffers
            .lock()
            .expect("buffer store poisoned")
            .handles()
            .count()
    }
}
