//! The tray (menu bar) icon: show the window, sync now, open the folder, quit.

use std::sync::mpsc::{Receiver, Sender, channel};

use craft_proton_sync_core::APP_NAME;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::icon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Toggle,
    Show,
    SyncNow,
    OpenFolder,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    rx: Receiver<TrayAction>,
    tooltip: String,
}

/// Names the icon on Linux, where StatusNotifierItem hosts otherwise show tray-icon's default id,
/// a number like "61965-1".
fn identity(builder: TrayIconBuilder) -> TrayIconBuilder {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        builder
    } else {
        builder.with_id("craft-proton-sync").with_title(APP_NAME)
    }
}

impl Tray {
    /// Creates the tray icon. `wake` repaints the UI so it sees the action, even when hidden.
    /// `on_quit` runs when Quit is picked, before the UI can see it: the UI may never get to.
    pub fn new(wake: impl Fn() + Send + Sync + Clone + 'static, on_quit: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let show = MenuItem::new(format!("Show {APP_NAME}"), true, None);
        let sync = MenuItem::new("Sync Now", true, None);
        let folder = MenuItem::new("Open ArtCraft Folder", true, None);
        let quit = MenuItem::new(format!("Quit {APP_NAME}"), true, None);
        let menu = Menu::new();
        menu.append_items(&[&show, &sync, &folder, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| e.to_string())?;
        const SIZE: u32 = 64;
        let image = tray_icon::Icon::from_rgba(icon::rgba(SIZE), SIZE, SIZE).map_err(|e| e.to_string())?;
        let icon = identity(TrayIconBuilder::new())
            .with_menu(Box::new(menu))
            .with_icon(image)
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .with_tooltip(APP_NAME)
            .build()
            .map_err(|e| e.to_string())?;
        let (tx, rx): (Sender<TrayAction>, Receiver<TrayAction>) = channel();
        {
            let (tx, wake) = (tx.clone(), wake.clone());
            let items = [
                (show.id().clone(), TrayAction::Show),
                (sync.id().clone(), TrayAction::SyncNow),
                (folder.id().clone(), TrayAction::OpenFolder),
                (quit.id().clone(), TrayAction::Quit),
            ];
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let Some((_, action)) = items.iter().find(|(id, _)| *id == e.id) else {
                    return;
                };
                if *action == TrayAction::Quit {
                    on_quit();
                }
                let _ = tx.send(*action);
                wake();
            }));
        }
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = tx.send(TrayAction::Toggle);
                wake();
            }
        }));
        Ok(Self {
            icon,
            rx,
            tooltip: APP_NAME.to_string(),
        })
    }

    pub fn actions(&self) -> Vec<TrayAction> {
        self.rx.try_iter().collect()
    }

    /// Shows the status when the pointer rests on the icon.
    pub fn set_status(&mut self, status: &str) {
        let tip = format!("{APP_NAME} · {status}");
        if tip != self.tooltip {
            let _ = self.icon.set_tooltip(Some(&tip));
            self.tooltip = tip;
        }
    }
}
