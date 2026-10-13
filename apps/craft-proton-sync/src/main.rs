//! Craft Proton Sync: keeps what you save from the Craft apps in step with Proton Drive.
//!
//! The window can close to the tray (menu bar) and keep syncing, or quit entirely.

#![cfg_attr(windows, windows_subsystem = "windows")]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

mod icon;
mod tray;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use craft_proton_sync_core::APP_NAME;
use craft_proton_sync_core::protocol::{Backend, Command, Phase};
use craft_proton_sync_core::settings::Settings;
use craft_proton_sync_native::paths::Paths;
use craft_proton_sync_native::single_instance::{self, Claim};
use craft_proton_sync_native::{Engine, login, open, settings_store};
use craft_proton_sync_ui_egui::theme::{self, Tokens};
use craft_proton_sync_ui_egui::{Host, SyncUi};

use tray::{Tray, TrayAction};

const APP_ID: &str = "craft-proton-sync";
/// How long a tray Quit may wait for the window to take it up before the app exits anyway.
const QUIT_GRACE: Duration = Duration::from_secs(2);

/// Repaints the window, from any thread, once there is one.
#[derive(Clone, Default)]
struct Waker {
    window: Arc<Mutex<Option<egui::Context>>>,
    quit_pending: Arc<AtomicBool>,
}

impl Waker {
    fn wake(&self) {
        if let Some(ctx) = self.window.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            ctx.request_repaint();
        }
    }

    fn set_window(&self, ctx: Option<egui::Context>) {
        *self.window.lock().unwrap_or_else(PoisonError::into_inner) = ctx;
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("craft-proton-sync {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let hidden = args.iter().any(|a| a == login::HIDDEN_FLAG);

    let paths = match Paths::new() {
        Ok(paths) => paths,
        Err(e) => {
            log::error!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let _instance = match single_instance::claim(&paths.instance_lock()) {
        Ok(Claim::Primary(lock)) => Some(lock),
        Ok(Claim::Secondary) => {
            log::info!("{APP_NAME} is already running");
            return ExitCode::SUCCESS;
        }
        // Better to run without the guard than not at all.
        Err(e) => {
            log::warn!("{e}");
            None
        }
    };
    let settings = settings_store::load(&paths.settings());
    let local_root = match paths.local_root(&settings) {
        Ok(root) => root,
        Err(e) => {
            log::error!("{e}");
            return ExitCode::FAILURE;
        }
    };
    log::info!("{APP_NAME} {}: syncing {}", env!("CARGO_PKG_VERSION"), local_root.display());

    let waker = Waker::default();
    let engine = {
        let waker = waker.clone();
        Arc::new(Engine::start(paths.clone(), &settings, local_root, move || waker.wake()))
    };

    let result = {
        let (engine, waker) = (engine.clone(), waker.clone());
        eframe::run_native(
            APP_NAME,
            native_options(hidden),
            Box::new(move |cc| {
                waker.set_window(Some(cc.egui_ctx.clone()));
                theme::install(&cc.egui_ctx);
                Ok(Box::new(Shell::new(cc, engine, waker, settings, paths.settings(), hidden)))
            }),
        )
    };
    waker.set_window(None);
    engine.shutdown();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn native_options(hidden: bool) -> eframe::NativeOptions {
    const ICON: u32 = 256;
    let viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_app_id(APP_ID)
        .with_inner_size([460.0, 680.0])
        .with_min_inner_size([380.0, 420.0])
        .with_icon(egui::IconData {
            rgba: icon::rgba(ICON),
            width: ICON,
            height: ICON,
        })
        .with_visible(!hidden);
    eframe::NativeOptions {
        viewport,
        wgpu_options: wgpu_options(),
        ..Default::default()
    }
}

/// wgpu's backends. On Windows that is Direct3D 12 with GL as the fallback, not Vulkan, and
/// shaders compiled with FXC, which is part of Windows: ArtCraft Launcher found Vulkan crashing
/// in the driver on laptops with two GPUs, and wgpu's default shader compiler failing where no
/// matching `dxil.dll` is installed. `WGPU_BACKEND` and `WGPU_DX12_COMPILER` still override.
#[cfg(windows)]
fn wgpu_options() -> eframe::WgpuConfiguration {
    use eframe::wgpu::{Backends, Dx12Compiler};
    let mut config = eframe::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        setup.instance_descriptor.backends = Backends::from_env().unwrap_or(Backends::DX12 | Backends::GL);
        setup.instance_descriptor.backend_options.dx12.shader_compiler = Dx12Compiler::from_env().unwrap_or(Dx12Compiler::Fxc);
    }
    config
}

#[cfg(not(windows))]
fn wgpu_options() -> eframe::WgpuConfiguration {
    eframe::WgpuConfiguration::default()
}

/// Whether the window system can hide a window and show it again. Wayland can't, so there the
/// close button quits.
fn window_can_hide(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    let wayland = || {
        ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"]
            .iter()
            .any(|name| env(name).is_some_and(|v| !v.is_empty()))
    };
    !(cfg!(all(unix, not(target_os = "macos"))) && wayland())
}

struct DesktopHost {
    ctx: egui::Context,
    has_tray: bool,
    settings_path: PathBuf,
}

impl Host for DesktopHost {
    fn open_folder(&mut self, path: &Path) {
        if let Err(e) = std::fs::create_dir_all(path)
            .map_err(|e| e.to_string())
            .and_then(|()| open::folder(path))
        {
            log::warn!("{e}");
        }
    }

    fn open_sign_in_page(&mut self, url: &str) {
        if let Err(e) = open::sign_in_page(url) {
            log::warn!("{e}");
        }
    }

    fn copy_text(&mut self, text: &str) {
        self.ctx.copy_text(text.to_string());
    }

    fn login_item(&self) -> Option<bool> {
        Some(login::is_enabled())
    }

    fn set_login_item(&mut self, on: bool) -> Result<(), String> {
        login::set_enabled(on)
    }

    fn has_tray(&self) -> bool {
        self.has_tray
    }

    fn settings_changed(&mut self, settings: &Settings) {
        if let Err(e) = settings_store::save(&self.settings_path, settings) {
            log::warn!("{e}");
        }
    }

    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

struct Shell {
    ui: SyncUi,
    engine: Arc<Engine>,
    host: DesktopHost,
    waker: Waker,
    tray: Option<Tray>,
    window_hides: bool,
    visible: bool,
    quitting: bool,
}

impl Shell {
    fn new(
        cc: &eframe::CreationContext<'_>,
        engine: Arc<Engine>,
        waker: Waker,
        settings: Settings,
        settings_path: PathBuf,
        hidden: bool,
    ) -> Self {
        let window_hides = window_can_hide(|name| std::env::var_os(name));
        let tray = {
            let (wake, quit, engine) = (waker.clone(), waker.clone(), engine.clone());
            Tray::new(
                move || wake.wake(),
                // A hidden window may get no frames, so the quit would wait until it is shown: if no
                // frame has taken it up in time, stop the sync and exit from here.
                move || {
                    quit.quit_pending.store(true, Ordering::SeqCst);
                    let (quit, engine) = (quit.clone(), engine.clone());
                    std::thread::spawn(move || {
                        std::thread::sleep(QUIT_GRACE);
                        if quit.quit_pending.load(Ordering::SeqCst) {
                            log::warn!("the window did not answer a quit from the tray: exiting");
                            engine.shutdown();
                            std::process::exit(0);
                        }
                    });
                },
            )
        };
        let tray = match tray {
            Ok(tray) => Some(tray),
            Err(e) => {
                log::warn!("no tray icon: {e}");
                None
            }
        };
        // Started hidden at login but there's no tray to come back from: show the window.
        let visible = !hidden || tray.is_none() || !window_hides;
        if visible && hidden {
            cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }
        let host = DesktopHost {
            ctx: cc.egui_ctx.clone(),
            has_tray: tray.is_some() && window_hides,
            settings_path,
        };
        Self {
            ui: SyncUi::new(settings),
            engine,
            host,
            waker,
            tray,
            window_hides,
            visible,
            quitting: false,
        }
    }

    fn show(&mut self, ctx: &egui::Context) {
        self.visible = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide(&mut self, ctx: &egui::Context) {
        self.visible = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        self.waker.quit_pending.store(false, Ordering::SeqCst);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let snapshot = self.engine.snapshot();
        for action in self.tray.as_ref().map(Tray::actions).unwrap_or_default() {
            match action {
                TrayAction::Toggle if self.visible && self.window_hides => self.hide(ctx),
                TrayAction::Toggle | TrayAction::Show => self.show(ctx),
                TrayAction::SyncNow => self.engine.send(Command::SyncNow),
                TrayAction::OpenFolder => self.host.open_folder(&snapshot.local_root),
                TrayAction::Quit => self.quit(ctx),
            }
        }
        if let Some(tray) = &mut self.tray {
            tray.set_status(craft_proton_sync_ui_egui::status(&snapshot, &Tokens::get(ctx)).0);
        }
        // The close button puts the window away in the tray unless the user chose otherwise.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.host.has_tray && self.ui.settings.close_to_tray {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide(ctx);
        }
        // "Last synced 3 minutes ago" keeps counting while the window is open.
        if self.visible && matches!(snapshot.phase, Phase::Idle | Phase::SyncFailed) {
            ctx.request_repaint_after(Duration::from_secs(30));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui.ui(ui, self.engine.as_ref(), &mut self.host);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        (if visuals.dark_mode { Tokens::DARK.bg } else { Tokens::LIGHT.bg }).to_normalized_gamma_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wayland_window_cannot_hide() {
        let wayland = |name: &str| (name == "WAYLAND_DISPLAY").then(|| "wayland-0".into());
        assert_eq!(window_can_hide(wayland), !cfg!(all(unix, not(target_os = "macos"))));
    }

    #[test]
    fn other_windows_hide() {
        assert!(window_can_hide(|_| None));
    }
}
