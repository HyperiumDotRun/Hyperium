use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        let _ = res.compile();
    }

    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");

    let hash = source_fingerprint(Path::new("src"));
    println!("cargo:rustc-env=HYPERIUM_BUILD_HASH={}", &format!("{hash:016x}")[..7]);

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, hh, mm) = civil(secs);
    println!("cargo:rustc-env=HYPERIUM_BUILD_DATE={y:04}-{mo:02}-{d:02} {hh:02}:{mm:02} UTC");

}

fn source_fingerprint(dir: &Path) -> u64 {
    let mut files: Vec<PathBuf> = Vec::new();
    collect(dir, &mut files);
    files.sort();

    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut fnv = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        fnv(f.to_string_lossy().as_bytes());
        fnv(b"\0");
        if let Ok(contents) = fs::read(f) {
            fnv(&contents);
        }
    }
    h
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn civil(secs: u64) -> (i64, u32, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let sod = secs % 86400;
    let hh = (sod / 3600) as u32;
    let mm = ((sod % 3600) / 60) as u32;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hh, mm)
}
