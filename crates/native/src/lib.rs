//! Craft Proton Sync's desktop backend.
//!
//! - [`install`]: downloads the pinned Proton Drive CLI and checks it before it is ever run.
//! - [`drive`]: runs the CLI, one command or a whole session.
//! - [`account`]: signing in to and out of Proton Drive.
//! - [`sync`]: one two-way sync of the folder.
//! - [`engine`]: the thread that watches the folder and decides when to sync.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod account;
pub mod drive;
pub mod engine;
pub mod install;
pub mod login;
pub mod open;
pub mod paths;
pub mod settings_store;
pub mod single_instance;
pub mod sync;

pub use engine::Engine;

use std::sync::{Mutex, MutexGuard, PoisonError};

/// A lock that outlives a panic on another thread: the data is plain state, still good to read.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
