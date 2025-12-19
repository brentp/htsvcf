//! V8 runtime initialization and global locking.
//!
//! This module manages the V8 JavaScript engine lifecycle, which is global
//! process state. V8 must be initialized exactly once per process, and
//! concurrent isolate creation from multiple threads must be serialized.
//!
//! # Thread Safety
//!
//! V8 isolates are single-threaded, but the platform itself requires careful
//! coordination. Use [`v8_lock()`] before creating or entering isolates from
//! multiple threads.
//!
//! # Initialization
//!
//! Call [`ensure_v8_initialized()`] before any V8 operations. It is safe to
//! call multiple times; initialization only happens once.
//!
//! # V8 Flags
//!
//! The runtime enables `--expose-gc` to allow explicit garbage collection
//! during benchmarking and diagnostics.

use std::sync::{Mutex, OnceLock};

pub(crate) const V8_FLAGS: &str = "--no_freeze_flags_after_init --expose-gc";

static V8_LOCK: Mutex<()> = Mutex::new(());
static PLATFORM: OnceLock<v8::SharedRef<v8::Platform>> = OnceLock::new();

/// Acquire the global V8 lock.
///
/// V8 is effectively global process state; the lock ensures callers don't
/// concurrently create/enter isolates from multiple threads.
pub fn v8_lock() -> std::sync::MutexGuard<'static, ()> {
    V8_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Initialize V8 once per process and return the shared platform.
///
/// This also enables `--expose-gc` so the embedder can trigger GC during
/// benchmarking/diagnostics.
pub fn ensure_v8_initialized() -> &'static v8::SharedRef<v8::Platform> {
    PLATFORM.get_or_init(|| {
        let platform = v8::new_unprotected_default_platform(0, false).make_shared();
        v8::V8::set_flags_from_string(V8_FLAGS);
        v8::V8::initialize_platform(platform.clone());
        v8::cppgc::initialize_process(platform.clone());
        v8::V8::initialize();
        platform
    })
}
