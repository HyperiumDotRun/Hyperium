use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct Recorder {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
}

impl Recorder {
    pub fn start() -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_input_device()?;
        let config = device.default_input_config().ok()?;
        let sample_rate = config.sample_rate();
        let channels = config.channels();
        let fmt = config.sample_format();
        let stream_cfg: cpal::StreamConfig = config.into();

        let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
        let on_err = |e| eprintln!("mic stream error: {e}");

        let buf = samples.clone();
        let stream = match fmt {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_cfg,
                move |data: &[f32], _: &_| buf.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(data),
                on_err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &stream_cfg,
                move |data: &[i16], _: &_| {
                    let mut b = buf.lock().unwrap_or_else(|e| e.into_inner());
                    b.extend(data.iter().map(|&s| s as f32 / 32768.0));
                },
                on_err,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &stream_cfg,
                move |data: &[u16], _: &_| {
                    let mut b = buf.lock().unwrap_or_else(|e| e.into_inner());
                    b.extend(data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0));
                },
                on_err,
                None,
            ),
            _ => return None,
        }
        .ok()?;
        stream.play().ok()?;
        Some(Self { stream, samples, sample_rate, channels })
    }

    pub fn finish(self) -> Vec<f32> {
        let raw = self.samples.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (sr, ch) = (self.sample_rate, self.channels);
        drop(self.stream);
        to_16k_mono(&raw, sr, ch)
    }
}

fn to_16k_mono(samples: &[f32], sample_rate: u32, channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let mono: Vec<f32> = if ch == 1 {
        samples.to_vec()
    } else {
        samples.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect()
    };
    if sample_rate == 16_000 || mono.is_empty() {
        return mono;
    }
    let ratio = 16_000.0 / sample_rate as f32;
    let out_len = (mono.len() as f32 * ratio) as usize;
    let last = mono.len() - 1;
    (0..out_len)
        .map(|i| {
            let src = i as f32 / ratio;
            let i0 = (src.floor() as usize).min(last);
            let i1 = (i0 + 1).min(last);
            let frac = src - i0 as f32;
            mono[i0] * (1.0 - frac) + mono[i1] * frac
        })
        .collect()
}

pub fn has_mic() -> bool {
    cpal::default_host().default_input_device().is_some()
}

pub fn models_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("models")
}

pub fn find_model(config_dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(models_dir(config_dir)).ok()?.flatten() {
        let p = entry.path();
        let is_ggml = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("bin"))
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("ggml-"));
        if is_ggml {
            let sz = entry.metadata().map(|m| m.len()).unwrap_or(0);
            if best.as_ref().is_none_or(|(b, _)| sz > *b) {
                best = Some((sz, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

pub fn find_cli(config_dir: &Path) -> Option<PathBuf> {
    let dir = models_dir(config_dir);
    for name in ["whisper-cli.exe", "main.exe", "whisper-cli", "main"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    let out = quiet("where").arg("whisper-cli").output().ok()?;
    if out.status.success() {
        let line = String::from_utf8_lossy(&out.stdout);
        let first = line.lines().next()?.trim();
        if !first.is_empty() {
            return Some(PathBuf::from(first));
        }
    }
    None
}

pub fn transcribe(config_dir: &Path, audio: &[f32]) -> Result<String, String> {
    if audio.len() < 16_000 / 5 {
        return Err("clip trop court".into());
    }
    if find_server(config_dir).is_some()
        && let Some(port) = ensure_server(config_dir)
    {
        return transcribe_via_server(port, audio);
    }
    match (find_model(config_dir), find_cli(config_dir)) {
        (Some(model), Some(cli)) => transcribe_via_cli(&model, &cli, audio),
        _ => Err("whisper-cli or model missing (⚙ → AI).".into()),
    }
}

fn transcribe_via_cli(model: &Path, cli: &Path, audio: &[f32]) -> Result<String, String> {
    let wav = write_temp_wav(audio).map_err(|e| format!("wav temporaire: {e}"))?;

    let threads = std::thread::available_parallelism().map(|n| n.get().max(1)).unwrap_or(4);
    let out = quiet(cli)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(&wav)
        .args(["-l", "auto", "-nt", "-t", &threads.to_string()])
        .output();
    let _ = std::fs::remove_file(&wav);

    let out = out.map_err(|e| format!("whisper-cli illisible: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).rev().take(2).collect();
        return Err(format!("whisper-cli failed: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" · ")));
    }
    let text = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(text.trim().to_string())
}

fn quiet(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn wav_bytes(audio: &[f32]) -> Vec<u8> {
    const SR: u32 = 16_000;
    let data_len = (audio.len() * 2) as u32;
    let mut buf = Vec::with_capacity(44 + data_len as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_len).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&SR.to_le_bytes());
    buf.extend_from_slice(&(SR * 2).to_le_bytes());
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &s in audio {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf
}

fn write_temp_wav(audio: &[f32]) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir()
        .join(format!("hyperium_dump_{}.wav", N.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&path, wav_bytes(audio))?;
    Ok(path)
}

static SERVER: Mutex<Option<ServerHandle>> = Mutex::new(None);

struct ServerHandle {
    child: Child,
    port: u16,
    #[cfg(windows)]
    job: Option<isize>,
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            crate::terminal::close_job(job);
        }
        let _ = self.child.kill();
    }
}

pub fn find_server(config_dir: &Path) -> Option<PathBuf> {
    let dir = models_dir(config_dir);
    for name in ["whisper-server.exe", "server.exe", "whisper-server", "server"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    let out = quiet("where").arg("whisper-server").output().ok()?;
    if out.status.success() {
        let line = String::from_utf8_lossy(&out.stdout);
        let first = line.lines().next()?.trim();
        if !first.is_empty() {
            return Some(PathBuf::from(first));
        }
    }
    None
}

pub fn warm(config_dir: &Path) {
    if find_server(config_dir).is_some() {
        let _ = ensure_server(config_dir);
    }
}

fn ensure_server(config_dir: &Path) -> Option<u16> {
    let mut guard = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = guard.as_mut() {
        match h.child.try_wait() {
            Ok(None) => return Some(h.port),
            _ => *guard = None,
        }
    }

    let server_bin = find_server(config_dir)?;
    let model = find_model(config_dir)?;
    let port = free_port()?;
    let threads = std::thread::available_parallelism().map(|n| n.get().max(1)).unwrap_or(4);

    let child = quiet(&server_bin)
        .arg("-m")
        .arg(&model)
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .args(["-t", &threads.to_string(), "-l", "auto"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    #[cfg(windows)]
    let job = crate::terminal::assign_to_job(child.id());

    let mut handle = ServerHandle {
        child,
        port,
        #[cfg(windows)]
        job,
    };

    if wait_ready(port, Duration::from_secs(120)) {
        *guard = Some(handle);
        Some(port)
    } else {
        let _ = handle.child.kill();
        #[cfg(windows)]
        if let Some(job) = handle.job.take() {
            crate::terminal::close_job(job);
        }
        std::mem::forget(handle);
        None
    }
}

fn transcribe_via_server(port: u16, audio: &[f32]) -> Result<String, String> {
    let wav = wav_bytes(audio);
    let boundary = "----hyperiumWhisperBoundaryf83Kd9";
    let mut body = Vec::with_capacity(wav.len() + 512);
    let mut field = |name: &str, value: &str| {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                .as_bytes(),
        );
    };
    field("response_format", "text");
    field("language", "auto");
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"dump.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let url = format!("http://127.0.0.1:{port}/inference");
    let resp = ureq::post(&url)
        .set("Content-Type", &format!("multipart/form-data; boundary={boundary}"))
        .timeout(Duration::from_secs(180))
        .send_bytes(&body)
        .map_err(|e| format!("whisper-server: {e}"))?;
    let text = resp.into_string().map_err(|e| format!("unreadable whisper response: {e}"))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string())
}

fn free_port() -> Option<u16> {
    std::net::TcpListener::bind("127.0.0.1:0").ok()?.local_addr().ok().map(|a| a.port())
}

fn wait_ready(port: u16, timeout: Duration) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}
