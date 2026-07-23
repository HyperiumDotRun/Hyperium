#[cfg(windows)]
const AUMID: &str = "Hyperium.Cockpit";

#[cfg(windows)]
static ICON_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

pub fn set_process_aumid() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        use windows::core::HSTRING;
        let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) };
    }
}

pub fn toast(title: &str, body: &str) {
    #[cfg(windows)]
    {
        use tauri_winrt_notification::{Duration, IconCrop, Toast};
        let mut toast = Toast::new(AUMID).title(title).text1(body).duration(Duration::Short);
        if let Some(p) = ICON_PATH.get() {
            toast = toast.icon(p, IconCrop::Square, "Hyperium");
        }
        let _ = toast.show();
    }
    #[cfg(not(windows))]
    {
        let _ = (title, body);
    }
}

pub fn ensure_registered(config_dir: &std::path::Path) {
    #[cfg(windows)]
    {
        let icon = config_dir.join("hyperium.png");
        write_icon_png(&icon);
        let key = format!(r"HKCU\Software\Classes\AppUserModelId\{AUMID}");
        reg_add(&key, "DisplayName", "Hyperium");
        if let Some(p) = icon.to_str() {
            reg_add(&key, "IconUri", p);
        }
        let _ = ICON_PATH.set(icon);
    }
    #[cfg(not(windows))]
    {
        let _ = config_dir;
    }
}

#[cfg(windows)]
fn reg_add(key: &str, name: &str, value: &str) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("reg")
        .args(["add", key, "/v", name, "/t", "REG_SZ", "/d", value, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(windows)]
fn write_icon_png(path: &std::path::Path) {
    let _ = std::fs::write(path, crate::icon::PNG);
}
