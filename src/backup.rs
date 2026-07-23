use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const KEEP: usize = 10;

pub fn default_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("backups")
}

fn dir_config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("backup_dir.txt")
}

pub fn configured_dir(config_dir: &Path) -> PathBuf {
    if let Ok(s) = std::fs::read_to_string(dir_config_path(config_dir)) {
        let s = s.trim();
        if !s.is_empty() {
            return PathBuf::from(s);
        }
    }
    default_dir(config_dir)
}

pub fn set_dir(config_dir: &Path, dir: &str) {
    let dir = dir.trim();
    let path = dir_config_path(config_dir);
    if dir.is_empty() || Path::new(dir) == default_dir(config_dir) {
        let _ = std::fs::remove_file(&path);
    } else {
        let _ = std::fs::write(&path, dir);
    }
}

struct Entry {
    src: PathBuf,
    arc: String,
}

pub struct Outcome {
    pub path: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

pub fn stamp() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let st = unsafe { GetLocalTime() };
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }
    #[cfg(not(windows))]
    {
        "00000000-000000".to_string()
    }
}

fn walk(dir: &Path, prefix: &str, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<Entry>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let p = ent.path();
        if !keep(&p) {
            continue;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        let child = format!("{prefix}/{name}");
        if p.is_dir() {
            walk(&p, &child, keep, out);
        } else if p.is_file() {
            out.push(Entry { src: p, arc: child });
        }
    }
}

fn collect(config_dir: &Path, project_dirs: &[PathBuf]) -> (Vec<Entry>, Vec<(String, String)>) {
    let mut out = Vec::new();
    let mut manifest = Vec::new();

    let models = config_dir.join("models");
    let backups = default_dir(config_dir);
    let machine = config_dir.join("machine.tsv");
    let fingerprint = fingerprint_path(config_dir);
    walk(
        config_dir,
        "config",
        &|p| p != models && p != backups && p != machine && p != fingerprint,
        &mut out,
    );

    let mut used: HashMap<String, u32> = HashMap::new();
    for dir in project_dirs {
        let notes = dir.join("hyperium-notes");
        if !notes.is_dir() {
            continue;
        }
        let base = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".to_string());
        let n = used.entry(base.clone()).or_insert(0);
        let label = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
        *n += 1;
        manifest.push((label.clone(), dir.to_string_lossy().into_owned()));
        walk(
            &notes,
            &format!("projects/{label}/hyperium-notes"),
            &|_| true,
            &mut out,
        );
    }

    out.sort_by(|a, b| a.arc.cmp(&b.arc));
    (out, manifest)
}

fn fingerprint(entries: &[Entry]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut buf = Vec::new();
    for e in entries {
        feed(e.arc.as_bytes());
        if let Ok(mut f) = std::fs::File::open(&e.src) {
            buf.clear();
            if f.read_to_end(&mut buf).is_ok() {
                feed(&buf);
            }
        }
    }
    h
}

fn fingerprint_path(config_dir: &Path) -> PathBuf {
    config_dir.join(".backup_fingerprint")
}

pub fn changed_since_last(config_dir: &Path, project_dirs: &[PathBuf]) -> bool {
    let (entries, _) = collect(config_dir, project_dirs);
    let fp = fingerprint(&entries);
    let last = std::fs::read_to_string(fingerprint_path(config_dir))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    last != Some(fp)
}

fn write_zip(path: &Path, entries: &[Entry], manifest: &[(String, String)]) -> io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    if !manifest.is_empty() {
        let body: String = manifest
            .iter()
            .map(|(label, abspath)| format!("{label}\t{abspath}\n"))
            .collect();
        zip.start_file("manifest.tsv", opts)?;
        zip.write_all(body.as_bytes())?;
    }

    let mut buf = Vec::new();
    for e in entries {
        let Ok(mut f) = std::fs::File::open(&e.src) else { continue };
        buf.clear();
        if f.read_to_end(&mut buf).is_err() {
            continue;
        }
        zip.start_file(e.arc.clone(), opts)?;
        zip.write_all(&buf)?;
    }
    zip.finish()?;
    Ok(())
}

fn rotate(dir: &Path, keep: usize) {
    if keep == 0 {
        return;
    }
    let mut zips = list(dir);
    if zips.len() > keep {
        let drop = zips.len() - keep;
        for p in zips.drain(..drop) {
            let _ = std::fs::remove_file(p);
        }
    }
}

pub fn list(dir: &Path) -> Vec<PathBuf> {
    let mut zips: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|n| n.starts_with("hyperium-backup-") && n.ends_with(".zip"))
                .unwrap_or(false)
        })
        .collect();
    zips.sort();
    zips
}

pub fn snapshot(
    config_dir: &Path,
    project_dirs: &[PathBuf],
    out_dir: &Path,
    keep: usize,
    stamp: &str,
) -> io::Result<Outcome> {
    let (entries, manifest) = collect(config_dir, project_dirs);
    std::fs::create_dir_all(out_dir)?;
    let path = out_dir.join(format!("hyperium-backup-{stamp}.zip"));
    write_zip(&path, &entries, &manifest)?;
    let _ = std::fs::write(fingerprint_path(config_dir), fingerprint(&entries).to_string());
    rotate(out_dir, keep);
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(Outcome {
        path,
        files: entries.len(),
        bytes,
    })
}

fn is_safe_relative(name: &str) -> bool {
    if name.is_empty() || name.contains('\\') || Path::new(name).is_absolute() {
        return false;
    }
    name.split('/').all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains(':'))
}

fn contained(base: &Path, target: PathBuf) -> Option<PathBuf> {
    if target.starts_with(base) {
        Some(target)
    } else {
        None
    }
}

fn is_known_project(proj: &Path, allowed: &HashSet<PathBuf>) -> bool {
    matches!(proj.canonicalize(), Ok(c) if allowed.contains(&c))
}

fn restore_target(
    config_dir: &Path,
    name: &str,
    manifest: &HashMap<String, PathBuf>,
    allowed: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    if name == "manifest.tsv" || !is_safe_relative(name) {
        return None;
    }
    if let Some(rest) = name.strip_prefix("config/") {
        return contained(config_dir, config_dir.join(rest));
    }
    if let Some(rest) = name.strip_prefix("projects/") {
        let (label, tail) = rest.split_once('/')?;
        let proj = manifest.get(label)?;
        if !is_known_project(proj, allowed) {
            return None;
        }
        return contained(proj, proj.join(tail));
    }
    None
}

pub fn restore(
    config_dir: &Path,
    zip_path: &Path,
    project_dirs: &[PathBuf],
) -> io::Result<usize> {
    let file = std::fs::File::open(zip_path)?;
    let mut zip = zip::ZipArchive::new(file)?;

    let allowed: HashSet<PathBuf> =
        project_dirs.iter().filter_map(|d| d.canonicalize().ok()).collect();

    let mut manifest: HashMap<String, PathBuf> = HashMap::new();
    {
        if let Ok(mut mf) = zip.by_name("manifest.tsv") {
            let mut s = String::new();
            let _ = mf.read_to_string(&mut s);
            for line in s.lines() {
                if let Some((label, abspath)) = line.split_once('\t') {
                    manifest.insert(label.to_string(), PathBuf::from(abspath));
                }
            }
        }
    }

    let mut count = 0usize;
    let mut buf = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().to_string();
        let Some(target) = restore_target(config_dir, &name, &manifest, &allowed) else {
            continue;
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        buf.clear();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&target, &buf)?;
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_restore_roundtrip_and_change_detection() {
        let base = std::env::temp_dir().join(format!("hyperium-bk-{}", stamp()));
        let cfg = base.join("config");
        let proj = base.join("myproject");
        let notes = proj.join("hyperium-notes");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(notes.join("log")).unwrap();

        std::fs::write(cfg.join("projects.tsv"), "A\tZ:\\a\n").unwrap();
        std::fs::write(cfg.join("command_memo.tsv"), "ls\tlist files\n").unwrap();
        std::fs::write(notes.join("NOTES.md"), "# notes\nhello\n").unwrap();
        std::fs::write(notes.join("log").join("d.md"), "log line\n").unwrap();

        let out = base.join("backups");
        let projects = vec![proj.clone()];

        let snap = snapshot(&cfg, &projects, &out, KEEP, "test").unwrap();
        assert_eq!(snap.files, 4, "config(2) + notes(2)");
        assert!(
            !changed_since_last(&cfg, &projects),
            "nothing changed since the snapshot - must skip"
        );

        std::fs::write(cfg.join("command_memo.tsv"), "ls -la\tlong list\n").unwrap();
        assert!(changed_since_last(&cfg, &projects));

        std::fs::write(cfg.join("projects.tsv"), "CORRUPT\n").unwrap();
        std::fs::remove_file(notes.join("NOTES.md")).unwrap();

        let n = restore(&cfg, &snap.path, &projects).unwrap();
        assert_eq!(n, 4);
        assert_eq!(
            std::fs::read_to_string(cfg.join("projects.tsv")).unwrap(),
            "A\tZ:\\a\n"
        );
        assert_eq!(
            std::fs::read_to_string(notes.join("NOTES.md")).unwrap(),
            "# notes\nhello\n",
            "notes restored to their original project folder via the manifest"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
