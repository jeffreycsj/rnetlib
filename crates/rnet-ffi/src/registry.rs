use crate::abi::RnetSlice;
use crate::abi::RNET_ABI_VERSION;
use crate::abi::RNET_E_INTERNAL_PANIC;
use crate::abi::RNET_OK;
use rnet_core::BufferStore;
use rnet_core::ErrorCode;
use rnet_core::HandleTable;
use rnet_core::Result;
use rnet_core::RnetError;
use rnet_observe::BoundedLogger;
use rnet_transport::NetworkRuntime;
use std::cell::Cell;
use std::cell::RefCell;
use std::ffi::{c_char, CString};
use std::net::IpAddr;
use std::net::SocketAddr;
use std::ops::Deref;
use std::panic::catch_unwind;
use std::panic::AssertUnwindSafe;
use std::slice;
use std::str;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Instant;

pub(crate) struct RuntimeEntry {
    pub(crate) network: NetworkRuntime,
    pub(crate) buffers: BufferStore,
    pub(crate) logger: Mutex<Option<BoundedLogger>>,
    pub(crate) metrics_log_interval_ms: AtomicU64,
    pub(crate) last_metrics_log: Mutex<Instant>,
    pub(crate) active_calls: AtomicUsize,
}

pub(crate) struct RuntimeLease(Arc<RuntimeEntry>);

impl Deref for RuntimeLease {
    type Target = RuntimeEntry;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        self.0.active_calls.fetch_sub(1, Ordering::Release);
    }
}

static RUNTIMES: OnceLock<Mutex<HandleTable<Arc<RuntimeEntry>>>> = OnceLock::new();

thread_local! {
    pub(crate) static IN_LOG_CALLBACK: Cell<bool> = const { Cell::new(false) };
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_last_error(message: &str) {
    let sanitized = message.replace('\0', "\\0");
    LAST_ERROR.with(|slot| {
        *slot.borrow_mut() = CString::new(sanitized).expect("NUL bytes were escaped");
    });
}

pub(crate) fn last_error_ptr() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

pub(crate) struct LogCallbackGuard;

impl Drop for LogCallbackGuard {
    fn drop(&mut self) {
        IN_LOG_CALLBACK.with(|flag| flag.set(false));
    }
}

pub(crate) fn runtimes() -> &'static Mutex<HandleTable<Arc<RuntimeEntry>>> {
    RUNTIMES.get_or_init(|| Mutex::new(HandleTable::new()))
}

pub(crate) fn runtime_entry(handle: u64) -> Result<RuntimeLease> {
    let table = runtimes().lock().expect("runtime table poisoned");
    let entry = table
        .get(handle)
        .cloned()
        .ok_or_else(|| RnetError::new(ErrorCode::InvalidHandle, "invalid runtime"))?;
    entry.active_calls.fetch_add(1, Ordering::AcqRel);
    Ok(RuntimeLease(entry))
}

pub(crate) unsafe fn parse_address(host: RnetSlice, port: u16) -> Result<SocketAddr> {
    unsafe {
        with_borrowed_slice(host, |bytes| {
            let text = str::from_utf8(bytes)
                .map_err(|_| RnetError::new(ErrorCode::InvalidArgument, "host is not UTF-8"))?;
            let ip: IpAddr = text.parse().map_err(|_| {
                RnetError::new(ErrorCode::InvalidArgument, "host must be a numeric IP")
            })?;
            Ok(SocketAddr::new(ip, port))
        })
    }
}

pub(crate) unsafe fn with_borrowed_slice<T>(
    value: RnetSlice,
    operation: impl FnOnce(&[u8]) -> Result<T>,
) -> Result<T> {
    if value.len == 0 {
        return operation(&[]);
    }
    if value.ptr.is_null() {
        return invalid_argument("non-empty slice has a null pointer");
    }
    let bytes = unsafe { slice::from_raw_parts(value.ptr, value.len) };
    operation(bytes)
}

pub(crate) unsafe fn copy_key(value: RnetSlice) -> Result<[u8; 32]> {
    unsafe {
        with_borrowed_slice(value, |bytes| {
            if bytes.len() != 32 {
                return invalid_argument("Noise keys must contain exactly 32 bytes");
            }
            let mut key = [0; 32];
            key.copy_from_slice(bytes);
            Ok(key)
        })
    }
}

pub(crate) fn validate_struct(struct_size: u32, abi_version: u32, required: usize) -> Result<()> {
    if struct_size < required as u32 || abi_version != RNET_ABI_VERSION {
        return invalid_argument("invalid struct_size or abi_version");
    }
    Ok(())
}

pub(crate) fn invalid_argument<T>(message: &str) -> Result<T> {
    Err(RnetError::new(ErrorCode::InvalidArgument, message))
}

pub(crate) fn invalid_state<T>(message: &str) -> Result<T> {
    Err(RnetError::new(ErrorCode::InvalidState, message))
}

pub(crate) fn ffi_status(operation: impl FnOnce() -> Result<()>) -> i32 {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(())) => {
            set_last_error("");
            RNET_OK
        }
        Ok(Err(error)) => {
            set_last_error(&error.to_string());
            error.code() as i32
        }
        Err(_) => {
            set_last_error("internal panic crossed the FFI boundary");
            RNET_E_INTERNAL_PANIC
        }
    }
}
