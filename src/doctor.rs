use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct EngineProbe {
    pub id: String,
    pub display: String,
    pub command: String,
    pub version_args: Vec<String>,
}

impl EngineProbe {
    fn new(id: &str, display: &str, command: &str, version_args: &[&str]) -> Self {
        Self {
            id: id.to_string(),
            display: display.to_string(),
            command: command.to_string(),
            version_args: version_args.iter().map(|s| s.to_string()).collect(),
        }
    }
}

#[derive(Clone)]
pub struct DetectedEngine {
    pub id: String,
    pub display: String,
    pub found: bool,
    pub version: Option<String>,
    pub path: Option<String>,
}

#[derive(Clone)]
pub struct MachineReport {
    pub scanned_at: SystemTime,
    pub engines: Vec<DetectedEngine>,
}

impl MachineReport {
    pub fn age_secs(&self) -> u64 {
        self.scanned_at.elapsed().map(|d| d.as_secs()).unwrap_or(0)
    }
}

pub fn cache_path(config_dir: &Path) -> PathBuf {
    config_dir.join("machine.tsv")
}

pub fn probes_path(config_dir: &Path) -> PathBuf {
    config_dir.join("engines.tsv")
}

pub fn default_probes() -> Vec<EngineProbe> {
    vec![
        EngineProbe::new("python", "Python", "python", &["--version"]),
        EngineProbe::new("python3", "Python 3", "python3", &["--version"]),
        EngineProbe::new("node", "Node.js", "node", &["--version"]),
        EngineProbe::new("go", "Go", "go", &["version"]),
        EngineProbe::new("rustc", "Rust (rustc)", "rustc", &["--version"]),
        EngineProbe::new("cargo", "Cargo", "cargo", &["--version"]),
        EngineProbe::new("java", "Java", "java", &["-version"]),
        EngineProbe::new("php", "PHP", "php", &["--version"]),
        EngineProbe::new("dotnet", ".NET", "dotnet", &["--version"]),
        EngineProbe::new("dart", "Dart", "dart", &["--version"]),
        EngineProbe::new("flutter", "Flutter", "flutter", &["--version"]),
        EngineProbe::new("adb", "Android adb", "adb", &["version"]),
        EngineProbe::new("ffmpeg", "ffmpeg", "ffmpeg", &["-version"]),
        EngineProbe::new("git", "Git", "git", &["--version"]),
        EngineProbe::new("docker", "Docker", "docker", &["--version"]),
        EngineProbe::new("deno", "Deno", "deno", &["--version"]),
        EngineProbe::new("bun", "Bun", "bun", &["--version"]),
        EngineProbe::new("ruby", "Ruby", "ruby", &["--version"]),
    ]
}

pub fn load_probes(path: &Path) -> Vec<EngineProbe> {
    let mut probes = default_probes();
    let Ok(content) = std::fs::read_to_string(path) else {
        return probes;
    };
    for line in content.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.split('\t');
        let (Some(id), Some(display), Some(command)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if id.is_empty() || command.is_empty() {
            continue;
        }
        let version_args: Vec<String> = f
            .next()
            .unwrap_or("--version")
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        let probe = EngineProbe {
            id: id.to_string(),
            display: if display.is_empty() { id.to_string() } else { display.to_string() },
            command: command.to_string(),
            version_args: if version_args.is_empty() {
                vec!["--version".to_string()]
            } else {
                version_args
            },
        };
        match probes.iter_mut().find(|p| p.id == probe.id) {
            Some(existing) => *existing = probe,
            None => probes.push(probe),
        }
    }
    probes
}

pub(crate) fn quiet_command(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn combined_output(out: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

fn run_version(command: &str, args: &[String], allow_fallback: bool) -> Option<String> {
    if let Ok(out) = quiet_command(command).args(args).output() {
        return Some(combined_output(&out));
    }
    #[cfg(windows)]
    if allow_fallback
        && let Ok(out) = quiet_command("cmd").arg("/c").arg(command).args(args).output()
    {
        return Some(combined_output(&out));
    }
    let _ = allow_fallback;
    None
}

pub fn probe_one(probe: &EngineProbe) -> DetectedEngine {
    let path = quiet_command("where").arg(&probe.command).output().ok().and_then(|o| {
        if o.status.success() {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
        } else {
            None
        }
    });

    let version = run_version(&probe.command, &probe.version_args, path.is_some())
        .as_deref()
        .and_then(extract_version);

    DetectedEngine {
        id: probe.id.clone(),
        display: probe.display.clone(),
        found: version.is_some() || path.is_some(),
        version,
        path,
    }
}

pub fn scan(probes: &[EngineProbe]) -> MachineReport {
    let engines = std::thread::scope(|s| {
        let handles: Vec<_> = probes.iter().map(|p| s.spawn(|| probe_one(p))).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| poisoned(""))).collect()
    });
    MachineReport { scanned_at: SystemTime::now(), engines }
}

fn poisoned(id: &str) -> DetectedEngine {
    DetectedEngine {
        id: id.to_string(),
        display: id.to_string(),
        found: false,
        version: None,
        path: None,
    }
}

fn extract_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_digit()) {
            let start = i;
            let mut seen_dot = false;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                if bytes[i] == b'.' {
                    seen_dot = true;
                }
                i += 1;
            }
            let tok = text[start..i].trim_end_matches('.');
            if seen_dot && tok.contains('.') {
                return Some(tok.to_string());
            }
        } else {
            i += 1;
        }
    }
    None
}

pub fn load_cache(path: &Path) -> Option<MachineReport> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let secs: u64 = lines.next()?.strip_prefix("scanned\t")?.trim().parse().ok()?;
    let scanned_at = UNIX_EPOCH + std::time::Duration::from_secs(secs);
    let mut engines = Vec::new();
    for line in lines {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let mut f = line.split('\t');
        let (Some(id), Some(display), Some(found)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let version = f.next().filter(|s| !s.is_empty()).map(|s| s.to_string());
        let path = f.next().filter(|s| !s.is_empty()).map(|s| s.to_string());
        engines.push(DetectedEngine {
            id: id.to_string(),
            display: display.to_string(),
            found: found == "1",
            version,
            path,
        });
    }
    Some(MachineReport { scanned_at, engines })
}

pub fn save_cache(path: &Path, report: &MachineReport) {
    let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    let secs = report.scanned_at.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut out = format!("scanned\t{secs}\n");
    for e in &report.engines {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            clean(&e.id),
            clean(&e.display),
            if e.found { "1" } else { "0" },
            clean(e.version.as_deref().unwrap_or("")),
            clean(e.path.as_deref().unwrap_or("")),
        ));
    }
    let _ = std::fs::write(path, out);
}
