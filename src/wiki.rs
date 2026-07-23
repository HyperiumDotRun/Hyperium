use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub fn has_wiki(project_path: &str) -> bool {
    if project_path.is_empty() {
        return false;
    }
    let root = Path::new(project_path);
    root.join("mkdocs.yml").is_file() || root.join("mkdocs.yaml").is_file()
}

const TEMPLATE_MKDOCS: &str = include_str!("../assets/wiki-template/mkdocs.yml");
const TEMPLATE_README: &str = include_str!("../assets/wiki-template/README.md");
const NAME_PLACEHOLDER: &str = "__PROJECT__";

pub fn create_wiki(project_path: &str, project_name: &str) -> Result<(), String> {
    let root = Path::new(project_path);
    if project_path.is_empty() || !root.is_dir() {
        return Err("project folder does not exist".into());
    }
    if has_wiki(project_path) {
        return Err("this project already has a wiki".into());
    }
    std::fs::write(root.join("mkdocs.yml"), TEMPLATE_MKDOCS.replace(NAME_PLACEHOLDER, project_name))
        .map_err(|e| format!("write mkdocs.yml: {e}"))?;
    let docs = root.join("docs");
    std::fs::create_dir_all(&docs).map_err(|e| format!("create docs/: {e}"))?;
    let readme = docs.join("README.md");
    if !readme.exists() {
        std::fs::write(&readme, TEMPLATE_README.replace(NAME_PLACEHOLDER, project_name))
            .map_err(|e| format!("write docs/README.md: {e}"))?;
    }
    Ok(())
}

fn mkdocs_available() -> bool {
    python_command()
        .args(["-m", "mkdocs", "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn ensure_mkdocs() -> bool {
    if mkdocs_available() {
        return true;
    }
    crate::notify::toast("Wiki", "Installing mkdocs-material (one-time, this can take a minute)...");
    let _ = python_command()
        .args(["-m", "pip", "install", "--user", "mkdocs-material"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let ok = mkdocs_available();
    crate::notify::toast(
        "Wiki",
        if ok {
            "mkdocs-material installed."
        } else {
            "Could not install mkdocs - is Python 3 + pip on PATH?"
        },
    );
    ok
}

struct WikiServer {
    path: String,
    child: Child,
    port: u16,
    #[cfg(windows)]
    job: Option<isize>,
}

impl Drop for WikiServer {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            crate::terminal::close_job(job);
        }
        let _ = self.child.kill();
    }
}

static SERVERS: Mutex<Vec<WikiServer>> = Mutex::new(Vec::new());

pub fn running_port(project_path: &str) -> Option<u16> {
    let mut servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
    let idx = servers.iter().position(|s| s.path == project_path)?;
    match servers[idx].child.try_wait() {
        Ok(None) => Some(servers[idx].port),
        _ => {
            servers.remove(idx);
            None
        }
    }
}

pub fn is_running(project_path: &str) -> bool {
    running_port(project_path).is_some()
}

pub fn toggle(project_path: &str, on_done: impl FnOnce() + Send + 'static) {
    let path = project_path.to_string();
    std::thread::spawn(move || {
        if let Some(port) = running_port(&path) {
            open_browser(port);
        } else if let Some(port) = start(&path) {
            open_browser(port);
        }
        on_done();
    });
}

pub fn stop(project_path: &str) -> bool {
    let mut servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(idx) = servers.iter().position(|s| s.path == project_path) {
        servers.remove(idx);
        true
    } else {
        false
    }
}

fn start(project_path: &str) -> Option<u16> {
    if !ensure_mkdocs() {
        return None;
    }
    let port = free_port()?;
    let child = python_command()
        .args(["-m", "mkdocs", "serve", "-a"])
        .arg(format!("127.0.0.1:{port}"))
        .current_dir(project_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    #[cfg(windows)]
    let job = crate::terminal::assign_to_job(child.id());

    let server = WikiServer {
        path: project_path.to_string(),
        child,
        port,
        #[cfg(windows)]
        job,
    };

    if wait_ready(port, Duration::from_secs(20)) {
        SERVERS.lock().unwrap_or_else(|e| e.into_inner()).push(server);
        Some(port)
    } else {
        drop(server);
        None
    }
}

fn python_command() -> std::process::Command {
    let mut cmd = std::process::Command::new("python");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn open_browser(port: u16) {
    let url = format!("http://127.0.0.1:{port}/");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer")
            .arg(&url)
            .creation_flags(0x0800_0000)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok().map(|a| a.port())
}

fn wait_ready(port: u16, timeout: Duration) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}
