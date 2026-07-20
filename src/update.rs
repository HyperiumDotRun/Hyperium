use std::ffi::OsString;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use crate::sync::AppRelease;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_HASH: &str = env!("HYPERIUM_BUILD_HASH");

pub fn is_update(rel: &AppRelease) -> bool {
    if rel.version.is_empty() {
        return false;
    }
    let server = semver(&rel.version);
    let local = semver(VERSION);
    server > local || (server == local && !rel.hash.is_empty() && rel.hash != BUILD_HASH)
}

fn semver(s: &str) -> (u64, u64, u64) {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let mut next = || it.next().and_then(|p| p.trim().parse::<u64>().ok()).unwrap_or(0);
    (next(), next(), next())
}

pub fn exe_from_zip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    for i in 0..zip.len() {
        let mut e = zip.by_index(i)?;
        if e.is_file() && e.name().to_ascii_lowercase().ends_with(".exe") {
            let mut buf = Vec::new();
            e.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no .exe inside the published zip",
    ))
}

fn with_suffix(exe: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = exe.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

pub fn cleanup_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = with_suffix(&exe, ".old");
    if !old.is_file() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..10 {
            if std::fs::remove_file(&old).is_ok() || !old.is_file() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    });
}

pub fn apply_and_relaunch(new_bytes: &[u8]) -> Result<(), String> {
    if new_bytes.len() < 1024 {
        return Err("downloaded build looks too small - aborting".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("can't find own exe: {e}"))?;
    let new_tmp = with_suffix(&exe, ".new");
    let old = with_suffix(&exe, ".old");

    std::fs::write(&new_tmp, new_bytes).map_err(|e| format!("write staged exe: {e}"))?;

    if let Err(e) = crate::authenticode::verify_trusted(&new_tmp) {
        let _ = std::fs::remove_file(&new_tmp);
        return Err(format!("refusing unverified update: {e}"));
    }

    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(&exe, &old) {
        let _ = std::fs::remove_file(&new_tmp);
        return Err(format!("move live exe aside: {e}"));
    }

    if let Err(e) = std::fs::rename(&new_tmp, &exe) {
        let _ = std::fs::rename(&old, &exe);
        let _ = std::fs::remove_file(&new_tmp);
        return Err(format!("install new exe: {e}"));
    }

    std::process::Command::new(&exe)
        .spawn()
        .map_err(|e| format!("relaunch failed (new exe is installed; just restart): {e}"))?;
    Ok(())
}

#[derive(Default)]
pub struct Shared {
    pub busy: bool,
    pub message: String,
    pub available: Option<AppRelease>,
    pub checked: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn semver_orders_and_compares() {
        assert!(semver("0.46.0") > semver("0.45.1"));
        assert!(semver("v1.2.3") > semver("1.2.2"));
        assert_eq!(semver("0.45.1"), (0, 45, 1));
    }

    #[test]
    fn is_update_logic() {
        let rel = |v: &str, h: &str| AppRelease {
            version: v.into(),
            hash: h.into(),
            ..Default::default()
        };
        assert!(is_update(&rel("999.0.0", "abcdef0")));
        assert!(is_update(&rel(VERSION, "different")));
        assert!(!is_update(&rel(VERSION, BUILD_HASH)));
        assert!(!is_update(&rel("0.0.1", "whatever")));
        assert!(!is_update(&rel("", "")));
    }

    #[test]
    fn exe_zip_roundtrip() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("hyperium.exe", opts).unwrap();
        zip.write_all(b"MZ-not-really-an-exe").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert_eq!(exe_from_zip(&bytes).unwrap(), b"MZ-not-really-an-exe");
    }
}
