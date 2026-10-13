//! Craft Proton Sync's interface: sign in, see what is synced where, and a few settings.
//!
//! The UI owns nothing but what the user is typing. Each frame it reads the engine's
//! [`Snapshot`] and sends [`Command`]s; the desktop shell does everything else through [`Host`].

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod theme;

use std::path::Path;

use craft_proton_sync_core::layout::{APP_FOLDERS, REMOTE_ROOT};
use craft_proton_sync_core::protocol::{Backend, Command, Phase, Snapshot};
use craft_proton_sync_core::settings::{MAX_POLL_MINUTES, MIN_POLL_MINUTES, Settings};
use craft_proton_sync_core::{APP_NAME, drive};
use egui::{Color32, RichText, Ui};

use theme::Tokens;

/// What the UI needs from the desktop around it.
pub trait Host {
    fn open_folder(&mut self, path: &Path);
    /// Opens Proton's sign-in page in the browser.
    fn open_sign_in_page(&mut self, url: &str);
    fn copy_text(&mut self, text: &str);
    /// Whether the app starts at login; `None` where that can't be set.
    fn login_item(&self) -> Option<bool>;
    fn set_login_item(&mut self, on: bool) -> Result<(), String>;
    fn has_tray(&self) -> bool;
    /// The settings changed and should be saved.
    fn settings_changed(&mut self, settings: &Settings);
    /// Seconds since the Unix epoch.
    fn now(&self) -> u64;
}

pub struct SyncUi {
    pub settings: Settings,
    /// A problem from the desktop, such as a login item that couldn't be set.
    host_error: Option<String>,
}

/// "3 minutes ago", for a time `seconds` in the past.
pub fn ago(seconds: u64) -> String {
    match seconds {
        0..=59 => "just now".into(),
        60..=119 => "1 minute ago".into(),
        120..=3599 => format!("{} minutes ago", seconds / 60),
        3600..=7199 => "1 hour ago".into(),
        7200..=86_399 => format!("{} hours ago", seconds / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

/// The status in a couple of words and its colour: the header's badge and the tray's tooltip.
pub fn status(snapshot: &Snapshot, t: &Tokens) -> (&'static str, Color32) {
    match snapshot.phase {
        Phase::Starting => ("Starting", t.muted),
        Phase::InstallingEngine { .. } => ("Setting up", t.accent_ink),
        Phase::EngineFailed => ("Setup failed", t.bad),
        Phase::SignedOut => ("Signed out", t.muted),
        Phase::SigningIn => ("Signing in", t.accent_ink),
        Phase::Idle => ("Up to date", t.good),
        Phase::Syncing => ("Syncing", t.accent_ink),
        Phase::Paused => ("Paused", t.warn),
        Phase::SyncFailed => ("Sync failed", t.bad),
    }
}

fn label(ui: &mut Ui, t: &Tokens, text: &str) -> egui::Response {
    ui.label(RichText::new(text.to_uppercase()).monospace().size(11.0).color(t.faint))
}

fn section(ui: &mut Ui, t: &Tokens, title: &str) {
    ui.add_space(12.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, t.hairline());
    ui.add_space(4.0);
    label(ui, t, title);
}

fn primary(ui: &mut Ui, t: &Tokens, enabled: bool, text: &str) -> egui::Response {
    let text = RichText::new(text.to_uppercase()).monospace().size(12.0).color(t.invert_fg);
    ui.add_enabled(enabled, egui::Button::new(text).fill(t.invert_bg))
}

fn outline(ui: &mut Ui, t: &Tokens, text: &str) -> egui::Response {
    ui.button(RichText::new(text.to_uppercase()).monospace().size(12.0).color(t.ink))
}

fn problem(ui: &mut Ui, t: &Tokens, text: &str) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, t.bad))
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(t.bad));
        });
}

impl SyncUi {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            host_error: None,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, backend: &dyn Backend, host: &mut dyn Host) {
        let t = Tokens::get(ui.ctx());
        let snapshot = backend.snapshot();
        egui::Frame::new().fill(t.bg).inner_margin(20).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            self.header(ui, &t, &snapshot);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match snapshot.phase {
                    Phase::Starting => {
                        ui.add_space(24.0);
                        ui.spinner();
                    }
                    Phase::InstallingEngine { received, total } => install(ui, &t, received, total),
                    Phase::EngineFailed => {
                        ui.add_space(16.0);
                        ui.label("Craft Proton Sync uses Proton's own Drive CLI to talk to Proton Drive, and couldn't download it.");
                        problem(ui, &t, snapshot.error.as_deref().unwrap_or("Something went wrong."));
                        if primary(ui, &t, true, "Try again").clicked() {
                            backend.send(Command::RetryInstall);
                        }
                    }
                    Phase::SignedOut | Phase::SigningIn => sign_in(ui, &t, &snapshot, backend, host),
                    Phase::Idle | Phase::Syncing | Phase::Paused | Phase::SyncFailed => self.synced(ui, &t, &snapshot, backend, host),
                });
        });
    }

    fn header(&self, ui: &mut Ui, t: &Tokens, snapshot: &Snapshot) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(APP_NAME).size(22.0).strong().color(t.ink_strong));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, color) = status(snapshot, t);
                egui::Frame::new()
                    .fill(t.sunken)
                    .stroke(t.hairline())
                    .inner_margin(egui::Margin::symmetric(8, 3))
                    .show(ui, |ui| {
                        ui.label(RichText::new(text.to_uppercase()).monospace().size(11.0).color(color));
                    });
                if matches!(snapshot.phase, Phase::Syncing | Phase::SigningIn) {
                    ui.spinner();
                }
            });
        });
    }

    fn synced(&mut self, ui: &mut Ui, t: &Tokens, snapshot: &Snapshot, backend: &dyn Backend, host: &mut dyn Host) {
        section(ui, t, "Account");
        ui.horizontal(|ui| {
            ui.label(snapshot.account.as_deref().unwrap_or("Proton account"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if outline(ui, t, "Sign out").clicked() {
                    backend.send(Command::SignOut);
                }
            });
        });

        section(ui, t, "Sync");
        let last = match (&snapshot.phase, snapshot.last_sync) {
            (Phase::Syncing, _) => "Syncing now…".to_string(),
            (_, Some(when)) => format!("Last synced {}", ago(host.now().saturating_sub(when))),
            (_, None) => "Not synced yet".to_string(),
        };
        ui.horizontal(|ui| {
            ui.label(last);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let paused = snapshot.phase == Phase::Paused;
                if outline(ui, t, if paused { "Resume" } else { "Pause" }).clicked() {
                    self.settings.paused = !paused;
                    host.settings_changed(&self.settings);
                    backend.send(Command::SetPaused(!paused));
                }
                if primary(ui, t, !paused && snapshot.phase != Phase::Syncing, "Sync now").clicked() {
                    backend.send(Command::SyncNow);
                }
            });
        });
        if let Some(error) = snapshot.error.as_deref().filter(|_| snapshot.phase == Phase::SyncFailed) {
            problem(ui, t, error);
        }

        section(ui, t, "Where files go");
        ui.horizontal(|ui| {
            ui.label(RichText::new(snapshot.local_root.display().to_string()).monospace().size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if outline(ui, t, "Open folder").clicked() {
                    host.open_folder(&snapshot.local_root);
                }
            });
        });
        ui.label(
            RichText::new("Save from a Craft app into its folder and it appears at the same path in Proton Drive.")
                .size(12.0)
                .color(t.muted),
        );
        egui::CollapsingHeader::new("Folder for each app").show(ui, |ui| {
            egui::Grid::new("app-folders").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                for app in APP_FOLDERS {
                    ui.label(
                        RichText::new(snapshot.local_root.join(app).display().to_string())
                            .monospace()
                            .size(11.0),
                    );
                    ui.label(
                        RichText::new(format!("→  /{REMOTE_ROOT}/{app}"))
                            .monospace()
                            .size(11.0)
                            .color(t.accent_ink),
                    );
                    ui.end_row();
                }
            });
        });

        section(ui, t, "Recent activity");
        if snapshot.activity.is_empty() {
            ui.label(RichText::new("Nothing yet.").color(t.muted));
        }
        for item in snapshot.activity.iter().take(8) {
            ui.horizontal(|ui| {
                let color = if matches!(item.action, "Deleted" | "Kept both") {
                    t.warn
                } else {
                    t.good
                };
                ui.label(RichText::new(item.action.to_uppercase()).monospace().size(11.0).color(color));
                ui.label(RichText::new(&item.path).size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(ago(host.now().saturating_sub(item.when))).size(11.0).color(t.faint));
                });
            });
        }

        self.settings(ui, t, backend, host);

        ui.add_space(12.0);
        ui.label(
            RichText::new(format!(
                "Syncing by Proton Drive CLI {}. Not affiliated with or endorsed by Proton AG.",
                drive::VERSION
            ))
            .size(11.0)
            .color(t.faint),
        );
    }

    fn settings(&mut self, ui: &mut Ui, t: &Tokens, backend: &dyn Backend, host: &mut dyn Host) {
        section(ui, t, "Settings");
        ui.horizontal(|ui| {
            ui.label("Check Proton Drive every");
            let mut minutes = self.settings.poll_minutes;
            ui.add(egui::DragValue::new(&mut minutes).range(MIN_POLL_MINUTES..=MAX_POLL_MINUTES));
            ui.label(if minutes == 1 { "minute" } else { "minutes" });
            if minutes != self.settings.poll_minutes {
                self.settings.poll_minutes = minutes;
                host.settings_changed(&self.settings);
                backend.send(Command::SetPollMinutes(minutes));
            }
        });
        if let Some(mut on) = host.login_item()
            && ui.checkbox(&mut on, "Start when I log in").changed()
        {
            self.host_error = host.set_login_item(on).err();
        }
        if host.has_tray()
            && ui
                .checkbox(
                    &mut self.settings.close_to_tray,
                    "Keep syncing in the tray when the window is closed",
                )
                .changed()
        {
            host.settings_changed(&self.settings);
        }
        if let Some(error) = &self.host_error {
            problem(ui, t, error);
        }
    }
}

fn sign_in(ui: &mut Ui, t: &Tokens, snapshot: &Snapshot, backend: &dyn Backend, host: &mut dyn Host) {
    section(ui, t, "Sign in to Proton Drive");
    ui.label(format!(
        "Files you save in Documents/{REMOTE_ROOT} are kept in step with the {REMOTE_ROOT} folder in your Proton Drive."
    ));
    ui.add_space(4.0);
    if let Some(error) = &snapshot.sign_in_error {
        problem(ui, t, error);
    }
    if snapshot.phase == Phase::SigningIn {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Finish signing in on Proton's page in your browser.");
        });
        if let Some(url) = &snapshot.sign_in_url {
            ui.label(
                RichText::new("The page didn't open, or you'd rather sign in on another device?")
                    .size(12.0)
                    .color(t.muted),
            );
            ui.horizontal(|ui| {
                if outline(ui, t, "Open the page again").clicked() {
                    host.open_sign_in_page(url);
                }
                if outline(ui, t, "Copy the link").clicked() {
                    host.copy_text(url);
                }
            });
        }
        ui.add_space(4.0);
        if outline(ui, t, "Cancel").clicked() {
            backend.send(Command::CancelSignIn);
        }
    } else if primary(ui, t, true, "Sign in with Proton").clicked() {
        backend.send(Command::SignIn);
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            "You sign in on Proton's own page, so this app never sees your password. The session Proton gives back is kept in your system's secure storage.",
        )
        .size(12.0)
        .color(t.muted),
    );
}

fn install(ui: &mut Ui, t: &Tokens, received: u64, total: Option<u64>) {
    section(ui, t, "Setting up");
    ui.label(format!(
        "Downloading Proton's Drive CLI {}, which talks to Proton Drive. This happens once.",
        drive::VERSION
    ));
    let megabytes = |bytes: u64| bytes as f32 / (1024.0 * 1024.0);
    match total.filter(|t| *t > 0) {
        Some(total) => {
            ui.add(egui::ProgressBar::new(received as f32 / total as f32).corner_radius(0));
            ui.label(
                RichText::new(format!("{:.1} of {:.1} MB", megabytes(received), megabytes(total)))
                    .size(12.0)
                    .color(t.muted),
            );
        }
        None => {
            ui.spinner();
            ui.label(RichText::new(format!("{:.1} MB", megabytes(received))).size(12.0).color(t.muted));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_the_way_people_say_them() {
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(125), "2 minutes ago");
        assert_eq!(ago(3600), "1 hour ago");
        assert_eq!(ago(3 * 3600 + 5), "3 hours ago");
        assert_eq!(ago(86_400), "yesterday");
        assert_eq!(ago(5 * 86_400), "5 days ago");
    }
}
