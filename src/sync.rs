use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const OFFICIAL_BASE: &str = "https://hyperium.run";

pub fn official_base(cfg: &Path) -> String {
    let o = read_trim(&cfg.join("official_url.txt"));
    let base = if o.is_empty() { OFFICIAL_BASE.to_string() } else { o };
    base.trim_end_matches('/').to_string()
}

fn read_trim(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default().trim().to_string()
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Badge {
    #[default]
    Unknown,
    Synced,
    PushNeeded,
    PullNeeded,
    Diverged,
    LocalOnly,
}

pub fn classify(local: &str, server: Option<&str>, last: Option<&str>) -> Badge {
    match server {
        None => Badge::LocalOnly,
        Some(s) if s == local => Badge::Synced,
        Some(s) => match last {
            Some(l) if l == local => Badge::PullNeeded,
            Some(l) if l == s => Badge::PushNeeded,
            _ => Badge::Diverged,
        },
    }
}

fn state_path(cfg: &Path) -> PathBuf {
    cfg.join("sync_state.tsv")
}

pub fn load_synced(cfg: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Ok(s) = std::fs::read_to_string(state_path(cfg)) {
        for line in s.lines() {
            if let Some((name, hash)) = line.split_once('\t') {
                out.insert(name.to_string(), hash.trim().to_string());
            }
        }
    }
    out
}

pub fn save_synced(cfg: &Path, name: &str, hash: &str) {
    let mut map = load_synced(cfg);
    map.insert(name.to_string(), hash.to_string());
    let body: String = map.iter().map(|(n, h)| format!("{n}\t{h}\n")).collect();
    let _ = std::fs::write(state_path(cfg), body);
}

#[derive(Default)]
pub struct Shared {
    pub busy: bool,
    pub message: String,
    pub server: HashMap<String, String>,
    pub fetched: bool,
}

pub fn dir_hash(dir: &Path) -> String {
    let mut files = Vec::new();
    collect(dir, dir, &mut files);
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut buf = Vec::new();
    for (rel, abs) in &files {
        feed(rel.as_bytes());
        feed(b"\0");
        if let Ok(mut f) = std::fs::File::open(abs) {
            buf.clear();
            if f.read_to_end(&mut buf).is_ok() {
                feed(&buf);
            }
        }
    }
    format!("{h:016x}")
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(root, &p, out);
        } else if p.is_file()
            && let Ok(rel) = p.strip_prefix(root)
        {
            out.push((rel.to_string_lossy().replace('\\', "/"), p));
        }
    }
}

pub fn zip_dir(dir: &Path) -> std::io::Result<Vec<u8>> {
    let mut files = Vec::new();
    collect(dir, dir, &mut files);
    files.sort();
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut buf = Vec::new();
    for (rel, abs) in &files {
        let Ok(mut f) = std::fs::File::open(abs) else { continue };
        buf.clear();
        if f.read_to_end(&mut buf).is_err() {
            continue;
        }
        zip.start_file(rel.clone(), opts)?;
        zip.write_all(&buf)?;
    }
    Ok(zip.finish()?.into_inner())
}

pub fn unzip_into(dir: &Path, bytes: &[u8]) -> std::io::Result<usize> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut n = 0usize;
    let mut buf = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_string();
        if Path::new(&name).is_absolute() || name.split(['/', '\\']).any(|c| c == "..") {
            continue;
        }
        let target = dir.join(&name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        buf.clear();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&target, &buf)?;
        n += 1;
    }
    Ok(n)
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(45))
        .build()
}

fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn get(url: &str, big: bool) -> Result<ureq::Response, String> {
    let ag = if big { agent_big() } else { agent() };
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(300));
        }
        match ag.get(url).call() {
            Ok(r) => return Ok(r),
            Err(ureq::Error::Status(code, r)) => {
                let detail: String =
                    r.into_string().unwrap_or_default().chars().take(200).collect();
                return Err(format!("server error {code}: {detail}"));
            }
            Err(e) => last = format!("network error: {e}"),
        }
    }
    Err(last)
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    Ok(s)
}

pub fn templates_catalog(cfg: &Path) -> Result<serde_json::Value, String> {
    let u = format!("{}/market/catalog.json", official_base(cfg));
    get(&u, false)?.into_json().map_err(|e| format!("bad catalogue: {e}"))
}

pub fn template_download(cfg: &Path, id: &str) -> Result<Vec<u8>, String> {
    let u = format!("{}/market/t/{}.md", official_base(cfg), enc(id));
    let resp = get(&u, false)?;
    let mut buf = Vec::new();
    resp.into_reader().read_to_end(&mut buf).map_err(|e| format!("read failed: {e}"))?;
    Ok(buf)
}

pub fn download_template_thumb(cfg: &Path, id: &str, dest: &Path) -> Result<u64, String> {
    let u = format!("{}/market/thumb/{}.png", official_base(cfg), enc(id));
    download_to_file(&u, dest, |_, _| {})
}

fn agent_big() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(300))
        .build()
}

#[derive(Clone, Default)]
pub struct AppRelease {
    pub version: String,
    pub hash: String,
    pub date: String,
    pub size: u64,
    pub sha256: String,
    pub file: String,
}

pub fn fetch_app_manifest(cfg: &Path) -> Result<Option<AppRelease>, String> {
    let u = format!("{}/updates/latest.json", official_base(cfg));
    let v: serde_json::Value =
        get(&u, false)?.into_json().map_err(|e| format!("bad app manifest: {e}"))?;
    let ver = v["version"].as_str().unwrap_or("");
    if ver.is_empty() {
        return Ok(None);
    }
    Ok(Some(AppRelease {
        version: ver.to_string(),
        hash: v["hash"].as_str().unwrap_or("").to_string(),
        date: v["date"].as_str().unwrap_or("").to_string(),
        size: v["size"].as_u64().unwrap_or(0),
        sha256: v["sha256"].as_str().unwrap_or("").to_string(),
        file: v["file"].as_str().unwrap_or("").to_string(),
    }))
}

pub fn download_app(cfg: &Path, rel: &AppRelease) -> Result<Vec<u8>, String> {
    let file = if rel.file.trim().is_empty() {
        format!("hyperium-{}.zip", rel.version)
    } else {
        rel.file.clone()
    };
    let u = format!("{}/updates/{}", official_base(cfg), file);
    let resp = get(&u, true)?;
    let mut buf = Vec::new();
    resp.into_reader().read_to_end(&mut buf).map_err(|e| format!("read failed: {e}"))?;
    Ok(buf)
}

#[derive(Clone, Default)]
pub struct WhisperBin {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Default)]
pub struct WhisperModel {
    pub id: String,
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub label: String,
}

#[derive(Clone, Default)]
pub struct WhisperManifest {
    pub bin: WhisperBin,
    pub models: Vec<WhisperModel>,
}

pub fn whisper_manifest(cfg: &Path) -> Result<WhisperManifest, String> {
    let u = format!("{}/whisper/manifest.json", official_base(cfg));
    let v: serde_json::Value =
        get(&u, false)?.into_json().map_err(|e| format!("bad whisper manifest: {e}"))?;
    let bin = &v["bin"];
    let models = v["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|m| WhisperModel {
                    id: m["id"].as_str().unwrap_or("").to_string(),
                    name: m["name"].as_str().unwrap_or("").to_string(),
                    url: m["url"].as_str().unwrap_or("").to_string(),
                    sha256: m["sha256"].as_str().unwrap_or("").to_string(),
                    size: m["size"].as_u64().unwrap_or(0),
                    label: m["label"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(WhisperManifest {
        bin: WhisperBin {
            file: bin["file"].as_str().unwrap_or("").to_string(),
            sha256: bin["sha256"].as_str().unwrap_or("").to_string(),
            size: bin["size"].as_u64().unwrap_or(0),
        },
        models,
    })
}

pub fn whisper_bin_url(cfg: &Path, file: &str) -> String {
    format!("{}/whisper/{}", official_base(cfg), file)
}

fn with_part(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

pub fn download_to_file(
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<u64, String> {
    let resp = get(url, true)?;
    let total =
        resp.header("Content-Length").and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = with_part(dest);
    let mut file =
        std::fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done: u64 = 0;
    on_progress(0, total);
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("read failed: {e}"));
            }
        };
        if let Err(e) = file.write_all(&buf[..n]) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("write failed: {e}"));
        }
        done += n as u64;
        on_progress(done, total);
    }
    file.flush().map_err(|e| format!("flush failed: {e}"))?;
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|e| format!("finalize {}: {e}", dest.display()))?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_roundtrip_and_hash_changes_with_content() {
        let base = std::env::temp_dir().join(format!("hyperium-sync-{}", std::process::id()));
        let notes = base.join("hyperium-notes");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(notes.join("log")).unwrap();
        std::fs::write(notes.join("NOTES.md"), "# notes\nhello\n").unwrap();
        std::fs::write(notes.join("log").join("d.md"), "log\n").unwrap();

        let h1 = dir_hash(&notes);
        let bytes = zip_dir(&notes).unwrap();

        let dest = base.join("restored").join("hyperium-notes");
        std::fs::create_dir_all(&dest).unwrap();
        let n = unzip_into(&dest, &bytes).unwrap();
        assert_eq!(n, 2);
        assert_eq!(dir_hash(&dest), h1, "same bytes -> same hash on another machine");

        std::fs::write(notes.join("NOTES.md"), "# notes\nchanged\n").unwrap();
        assert_ne!(dir_hash(&notes), h1);

        let _ = std::fs::remove_dir_all(&base);
    }
}
