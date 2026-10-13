//! Craft Proton Sync core: pure logic with no I/O.
//!
//! - [`layout`]: where a file saved by a Craft app lives locally and on Proton Drive.
//! - [`drive`]: the pinned Proton Drive CLI release, its command lines and reading its answers.
//! - [`plan`]: what a two-way sync does, from three pictures of the folder.
//! - [`protocol`]: what the UI and the sync engine say to each other.
//! - [`settings`]: the user's settings.
//! - [`update`]: the app's own releases: which is newer, which file is this computer's.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod drive;
pub mod layout;
pub mod plan;
pub mod protocol;
pub mod settings;
pub mod update;

pub const APP_NAME: &str = "Craft Proton Sync";
