pub enum TrayAction {
    None,
    Show,
    Quit,
}

#[cfg(windows)]
pub use imp::{build, take_action, Tray};

#[cfg(windows)]
mod imp {
    use super::TrayAction;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use eframe::egui;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};

    #[derive(Default)]
    struct Shared {
        show: AtomicBool,
        quit: AtomicBool,
    }

    pub struct Tray {
        _icon: TrayIcon,
        shared: Arc<Shared>,
    }

    pub fn build(ctx: egui::Context) -> Option<Tray> {
        let menu = Menu::new();
        let show = MenuItem::new("Show Hyperium", true, None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append(&show).ok()?;
        menu.append(&quit).ok()?;
        let show_id = show.id().clone();
        let quit_id = quit.id().clone();
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Hyperium")
            .with_icon(brand_icon())
            .build()
            .ok()?;

        let shared = Arc::new(Shared::default());

        {
            let shared = shared.clone();
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
                if ev.id == show_id {
                    shared.show.store(true, Ordering::SeqCst);
                } else if ev.id == quit_id {
                    shared.quit.store(true, Ordering::SeqCst);
                }
                ctx.request_repaint();
            }));
        }
        {
            let shared = shared.clone();
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, .. } = ev {
                    shared.show.store(true, Ordering::SeqCst);
                    ctx.request_repaint();
                }
            }));
        }

        Some(Tray { _icon: icon, shared })
    }

    pub fn take_action(tray: &Tray) -> TrayAction {
        if tray.shared.quit.swap(false, Ordering::SeqCst) {
            TrayAction::Quit
        } else if tray.shared.show.swap(false, Ordering::SeqCst) {
            TrayAction::Show
        } else {
            TrayAction::None
        }
    }

    fn brand_icon() -> tray_icon::Icon {
        if let Some((rgba, w, h)) = crate::icon::rgba_resized(32)
            && let Ok(icon) = tray_icon::Icon::from_rgba(rgba, w, h)
        {
            return icon;
        }
        const S: u32 = 32;
        let mut rgba = vec![0u8; (S * S * 4) as usize];
        for y in 0..S {
            for x in 0..S {
                let i = ((y * S + x) * 4) as usize;
                let m = 3;
                let inside = x >= m && x < S - m && y >= m && y < S - m;
                let corner = (x < m + 2 || x >= S - m - 2) && (y < m + 2 || y >= S - m - 2);
                if inside && !corner {
                    rgba[i] = 178;
                    rgba[i + 1] = 232;
                    rgba[i + 2] = 44;
                    rgba[i + 3] = 255;
                }
            }
        }
        tray_icon::Icon::from_rgba(rgba, S, S).expect("valid tray icon")
    }
}

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
pub fn build(_ctx: eframe::egui::Context) -> Option<Tray> {
    None
}

#[cfg(not(windows))]
pub fn take_action(_tray: &Tray) -> TrayAction {
    TrayAction::None
}
