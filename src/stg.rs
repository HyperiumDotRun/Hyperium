use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

const ADDR: &str = "127.0.0.1:52473";

pub fn resolve_target(arg: &str) -> Option<String> {
    let arg = arg.trim().trim_matches('"');
    if arg.is_empty() {
        return None;
    }
    let p = Path::new(arg);
    if p.is_dir() {
        return Some(arg.to_string());
    }
    if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("hyp") || e.eq_ignore_ascii_case("stg"))
    {
        let content = std::fs::read_to_string(p).ok()?;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let path = line.split('\t').next().unwrap_or(line).trim();
            if Path::new(path).is_dir() {
                return Some(path.to_string());
            }
        }
    }
    None
}

pub fn ensure_file(folder: &str) {
    let dir = Path::new(folder);
    let Some(name) = dir.file_name().and_then(|s| s.to_str()) else {
        return;
    };
    let file = dir.join(format!("{name}.hyp"));
    if file.exists() {
        return;
    }
    let body = format!(
        "# Hyperium project launcher - double-click to open this project in Hyperium.\n{folder}\n"
    );
    let _ = std::fs::write(file, body);
}

pub enum Role {
    Primary(TcpListener),
    Secondary,
}

pub fn acquire(payload: &str) -> Role {
    match TcpListener::bind(ADDR) {
        Ok(listener) => Role::Primary(listener),
        Err(_) => {
            if let Ok(mut s) = TcpStream::connect(ADDR) {
                let _ = s.write_all(payload.as_bytes());
            }
            Role::Secondary
        }
    }
}

pub fn serve<F: Fn(String) + Send + 'static>(listener: TcpListener, on_message: F) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = String::new();
            if s.read_to_string(&mut buf).is_ok() {
                on_message(buf);
            }
        }
    });
}

pub fn register_association(icon: Option<&Path>) {
    #[cfg(windows)]
    {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Some(exe) = exe.to_str() else {
            return;
        };
        let default_icon = match icon.and_then(|p| p.to_str()) {
            Some(ico) => ico.to_string(),
            None => format!("{exe},0"),
        };
        reg_default(r"HKCU\Software\Classes\.hyp", "Hyperium.Project");
        reg_default(r"HKCU\Software\Classes\Hyperium.Project", "Hyperium Project");
        reg_default(r"HKCU\Software\Classes\Hyperium.Project\DefaultIcon", &default_icon);
        reg_default(
            r"HKCU\Software\Classes\Hyperium.Project\shell\open\command",
            &format!("\"{exe}\" \"%1\""),
        );
    }
    #[cfg(not(windows))]
    {
        let _ = icon;
    }
}

#[cfg(windows)]
fn reg_default(key: &str, value: &str) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("reg")
        .args(["add", key, "/ve", "/d", value, "/f"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}
