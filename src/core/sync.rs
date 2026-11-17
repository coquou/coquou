//! Locking shared state.

use std::sync::{Mutex, MutexGuard};

/// Lock `m`, recovering the value if a thread panicked while holding it.
/// Everything Coquou guards with a mutex is plain data that stays
/// consistent between statements, so a panic elsewhere must not take the
/// pet, the tray or the settings down with it.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
