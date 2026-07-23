use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct FtpConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub dir: String,
    pub tls: bool,
    pub passphrase: String,
}

impl FtpConfig {
    pub fn connectable(&self) -> bool {
        !self.host.trim().is_empty() && !self.user.trim().is_empty()
    }
}

pub fn load_config(cfg: &Path) -> FtpConfig {
    let read = |n: &str| std::fs::read_to_string(cfg.join(n)).unwrap_or_default().trim().to_string();
    FtpConfig {
        host: read("ftp_host.txt"),
        port: read("ftp_port.txt").parse().unwrap_or(21),
        user: read("ftp_user.txt"),
        password: crate::secret::load_secret(&cfg.join("ftp_password")),
        dir: read("ftp_dir.txt"),
        tls: read("ftp_tls.txt") != "0",
        passphrase: crate::secret::load_secret(&cfg.join("sync_passphrase")),
    }
}

pub fn save_config(cfg: &Path, c: &FtpConfig) {
    let w = |n: &str, v: &str| {
        let _ = std::fs::write(cfg.join(n), v.trim());
    };
    w("ftp_host.txt", &c.host);
    w("ftp_port.txt", &c.port.to_string());
    w("ftp_user.txt", &c.user);
    w("ftp_dir.txt", &c.dir);
    w("ftp_tls.txt", if c.tls { "1" } else { "0" });
    crate::secret::save_secret(&cfg.join("ftp_password"), &c.password);
    crate::secret::save_secret(&cfg.join("sync_passphrase"), &c.passphrase);
}

fn remote_name(project: &str) -> String {
    let mut s = String::with_capacity(project.len() * 2 + 5);
    for b in project.bytes() {
        s.push_str(&format!("{b:02x}"));
    }
    s.push_str(".hypv");
    s
}

const MANIFEST: &str = "manifest.hypv";

type Plain = suppaftp::FtpStream;
type Secure = suppaftp::RustlsFtpStream;

enum Conn {
    Plain(Plain),
    Tls(Secure),
}

impl Conn {
    fn open(cfg: &FtpConfig) -> Result<Conn, String> {
        let addr = format!("{}:{}", cfg.host.trim(), cfg.port);
        if cfg.tls {
            let tls = rustls_config();
            let stream = Secure::connect(&addr).map_err(|e| format!("connect: {e}"))?;
            let mut stream = stream
                .into_secure(suppaftp::RustlsConnector::from(Arc::new(tls)), cfg.host.trim())
                .map_err(|e| format!("TLS handshake: {e}"))?;
            stream.login(cfg.user.trim(), &cfg.password).map_err(|e| format!("login: {e}"))?;
            let _ = stream.transfer_type(suppaftp::types::FileType::Binary);
            let mut c = Conn::Tls(stream);
            c.enter_dir(&cfg.dir)?;
            Ok(c)
        } else {
            let mut stream = Plain::connect(&addr).map_err(|e| format!("connect: {e}"))?;
            stream.login(cfg.user.trim(), &cfg.password).map_err(|e| format!("login: {e}"))?;
            let _ = stream.transfer_type(suppaftp::types::FileType::Binary);
            let mut c = Conn::Plain(stream);
            c.enter_dir(&cfg.dir)?;
            Ok(c)
        }
    }

    fn enter_dir(&mut self, dir: &str) -> Result<(), String> {
        let dir = dir.trim().trim_matches('/');
        if dir.is_empty() {
            return Ok(());
        }
        for seg in dir.split('/').filter(|s| !s.is_empty()) {
            if self.cwd(seg).is_err() {
                self.mkdir(seg);
                self.cwd(seg).map_err(|e| format!("cannot enter/create '{seg}': {e}"))?;
            }
        }
        Ok(())
    }

    fn cwd(&mut self, p: &str) -> Result<(), String> {
        match self {
            Conn::Plain(s) => s.cwd(p).map_err(|e| e.to_string()),
            Conn::Tls(s) => s.cwd(p).map_err(|e| e.to_string()),
        }
    }
    fn mkdir(&mut self, p: &str) {
        match self {
            Conn::Plain(s) => {
                let _ = s.mkdir(p);
            }
            Conn::Tls(s) => {
                let _ = s.mkdir(p);
            }
        }
    }
    fn put(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let mut cur = Cursor::new(bytes);
        match self {
            Conn::Plain(s) => s.put_file(name, &mut cur).map(|_| ()).map_err(|e| e.to_string()),
            Conn::Tls(s) => s.put_file(name, &mut cur).map(|_| ()).map_err(|e| e.to_string()),
        }
    }
    fn get_opt(&mut self, name: &str) -> Option<Vec<u8>> {
        let r = match self {
            Conn::Plain(s) => s.retr_as_buffer(name),
            Conn::Tls(s) => s.retr_as_buffer(name),
        };
        r.ok().map(|c| c.into_inner())
    }
    fn rm(&mut self, name: &str) {
        match self {
            Conn::Plain(s) => {
                let _ = s.rm(name);
            }
            Conn::Tls(s) => {
                let _ = s.rm(name);
            }
        }
    }
    fn quit(self) {
        match self {
            Conn::Plain(mut s) => {
                let _ = s.quit();
            }
            Conn::Tls(mut s) => {
                let _ = s.quit();
            }
        }
    }
}

fn rustls_config() -> suppaftp::rustls::ClientConfig {
    use suppaftp::rustls::{ClientConfig, RootCertStore};
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    ClientConfig::builder().with_root_certificates(roots).with_no_client_auth()
}

fn read_manifest_on(c: &mut Conn, passphrase: &str) -> Result<HashMap<String, String>, String> {
    let Some(blob) = c.get_opt(MANIFEST) else {
        return Ok(HashMap::new());
    };
    let plain = crate::vault::decrypt(passphrase, &blob)
        .ok_or("wrong sync passphrase (cannot read the server manifest)")?;
    let v: serde_json::Value =
        serde_json::from_slice(&plain).map_err(|e| format!("bad manifest: {e}"))?;
    let mut out = HashMap::new();
    if let Some(map) = v.as_object() {
        for (k, val) in map {
            if let Some(h) = val.as_str() {
                out.insert(k.clone(), h.to_string());
            }
        }
    }
    Ok(out)
}

fn write_manifest_on(
    c: &mut Conn,
    passphrase: &str,
    map: &HashMap<String, String>,
) -> Result<(), String> {
    let json = serde_json::to_vec(map).map_err(|e| e.to_string())?;
    let blob = crate::vault::encrypt(passphrase, &json).ok_or("encrypt failed (set a passphrase)")?;
    c.put(MANIFEST, &blob)
}

pub fn test_connection(cfg: &FtpConfig) -> Result<(), String> {
    let mut c = Conn::open(cfg)?;
    let probe = ".hyperium-write-test";
    c.put(probe, b"ok").map_err(|e| format!("folder not writable: {e}"))?;
    c.rm(probe);
    c.quit();
    Ok(())
}

pub fn read_manifest(cfg: &FtpConfig) -> Result<HashMap<String, String>, String> {
    if cfg.passphrase.is_empty() {
        return Err("set a sync passphrase in Settings -> Sync".into());
    }
    let mut c = Conn::open(cfg)?;
    let r = read_manifest_on(&mut c, &cfg.passphrase);
    c.quit();
    r
}

pub fn push(cfg: &FtpConfig, name: &str, hash: &str, plaintext_zip: &[u8]) -> Result<(), String> {
    if cfg.passphrase.is_empty() {
        return Err("set a sync passphrase in Settings -> Sync".into());
    }
    let blob = crate::vault::encrypt(&cfg.passphrase, plaintext_zip)
        .ok_or("encryption failed (set a passphrase)")?;
    let mut c = Conn::open(cfg)?;
    let r = (|| {
        c.put(&remote_name(name), &blob)?;
        let mut man = read_manifest_on(&mut c, &cfg.passphrase)?;
        man.insert(name.to_string(), hash.to_string());
        write_manifest_on(&mut c, &cfg.passphrase, &man)
    })();
    c.quit();
    r
}

pub fn pull(cfg: &FtpConfig, name: &str) -> Result<Vec<u8>, String> {
    if cfg.passphrase.is_empty() {
        return Err("set a sync passphrase in Settings -> Sync".into());
    }
    let mut c = Conn::open(cfg)?;
    let blob = c.get_opt(&remote_name(name));
    c.quit();
    let blob = blob.ok_or_else(|| format!("\"{name}\" is not on the server"))?;
    crate::vault::decrypt(&cfg.passphrase, &blob)
        .ok_or_else(|| "wrong sync passphrase (cannot decrypt)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_name_is_hex_and_safe() {
        assert_eq!(remote_name("_templates"), "5f74656d706c61746573.hypv");
        assert!(!remote_name("../evil").contains('/'));
    }

    #[test]
    fn manifest_json_roundtrips_through_vault() {
        let mut m = HashMap::new();
        m.insert("proj".to_string(), "abc123".to_string());
        let json = serde_json::to_vec(&m).unwrap();
        let blob = crate::vault::encrypt("pw", &json).unwrap();
        let plain = crate::vault::decrypt("pw", &blob).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(v["proj"], "abc123");
    }
}
