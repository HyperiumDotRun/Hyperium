use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
}

#[derive(Clone, Copy)]
pub enum Extra {
    Bool(bool),
    Str(&'static str),
    EmptyArr,
}

#[derive(Clone, Copy)]
pub struct Knob {
    pub label: &'static str,
    pub key: &'static str,
    pub values: &'static [&'static str],
    pub numeric: bool,
}

pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub provider: &'static str,
    pub about: &'static str,
    pub kind: Kind,
    pub aspects: &'static [&'static str],
    pub quality: Option<Knob>,
    pub duration: Option<Knob>,
    pub extra: &'static [(&'static str, Extra)],
    pub ext: &'static str,
}

pub const MODELS: &[Model] = &[
    Model {
        id: "gpt-image-2-text-to-image",
        label: "GPT Image 2",
        provider: "OpenAI",
        about: "Photorealism, crisp text, product shots - the default.",
        kind: Kind::Image,
        aspects: &["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "auto"],
        quality: Some(Knob { label: "Resolution", key: "resolution", values: &["1K", "2K", "4K"], numeric: false }),
        duration: None,
        extra: &[],
        ext: "png",
    },
    Model {
        id: "nano-banana-pro",
        label: "Nano Banana Pro",
        provider: "Google",
        about: "Fast, excellent quality/cost ratio.",
        kind: Kind::Image,
        aspects: &["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3", "4:5", "5:4", "21:9", "auto"],
        quality: Some(Knob { label: "Resolution", key: "resolution", values: &["1K", "2K", "4K"], numeric: false }),
        duration: None,
        extra: &[("output_format", Extra::Str("png"))],
        ext: "png",
    },
    Model {
        id: "flux-2/flex-text-to-image",
        label: "Flux 2",
        provider: "Black Forest Labs",
        about: "Creative, stylised, artistic.",
        kind: Kind::Image,
        aspects: &["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"],
        quality: Some(Knob { label: "Resolution", key: "resolution", values: &["1K", "2K"], numeric: false }),
        duration: None,
        extra: &[],
        ext: "png",
    },
    Model {
        id: "bytedance/seedance-2",
        label: "Seedance 2",
        provider: "ByteDance",
        about: "Fast, high-quality text-to-video with audio.",
        kind: Kind::Video,
        aspects: &["16:9", "9:16", "1:1", "4:3", "3:4", "21:9", "adaptive"],
        quality: Some(Knob { label: "Resolution", key: "resolution", values: &["480p", "720p", "1080p", "4k"], numeric: false }),
        duration: Some(Knob { label: "Duration", key: "duration", values: &["5", "10"], numeric: true }),
        extra: &[("generate_audio", Extra::Bool(true))],
        ext: "mp4",
    },
    Model {
        id: "kling-3.0/video",
        label: "Kling 3.0",
        provider: "Kling",
        about: "Premium cinematic text-to-video.",
        kind: Kind::Video,
        aspects: &["16:9", "9:16", "1:1"],
        quality: Some(Knob { label: "Mode", key: "mode", values: &["std", "pro", "4K"], numeric: false }),
        duration: Some(Knob { label: "Duration", key: "duration", values: &["5", "10"], numeric: false }),
        extra: &[
            ("sound", Extra::Bool(false)),
            ("multi_shots", Extra::Bool(false)),
            ("multi_prompt", Extra::EmptyArr),
        ],
        ext: "mp4",
    },
];

pub fn find_model(name: &str) -> Option<usize> {
    let n = norm(name);
    if n.is_empty() {
        return None;
    }
    MODELS
        .iter()
        .position(|m| m.id == name)
        .or_else(|| MODELS.iter().position(|m| norm(m.id).contains(&n) || norm(m.label).contains(&n)))
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

pub struct Request {
    pub model_idx: usize,
    pub prompt: String,
    pub aspect: String,
    pub quality: Option<String>,
    pub duration: Option<String>,
}

fn build_input(m: &Model, req: &Request) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("prompt".into(), serde_json::json!(req.prompt));
    if !req.aspect.is_empty() {
        map.insert("aspect_ratio".into(), serde_json::json!(req.aspect));
    }
    if let (Some(k), Some(v)) = (m.quality.as_ref(), req.quality.as_ref()) {
        map.insert(k.key.into(), serde_json::json!(v));
    }
    if let (Some(k), Some(v)) = (m.duration.as_ref(), req.duration.as_ref()) {
        let val = if k.numeric {
            serde_json::json!(v.parse::<i64>().unwrap_or(5))
        } else {
            serde_json::json!(v)
        };
        map.insert(k.key.into(), val);
    }
    for (key, val) in m.extra {
        let jv = match val {
            Extra::Bool(b) => serde_json::json!(b),
            Extra::Str(s) => serde_json::json!(s),
            Extra::EmptyArr => serde_json::json!([]),
        };
        map.insert((*key).into(), jv);
    }
    serde_json::Value::Object(map)
}

pub struct Saved {
    pub path: PathBuf,
}

const API_BASE: &str = "https://api.kie.ai/api/v1/jobs";

fn api_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(45))
        .build()
}

fn dl_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout(Duration::from_secs(300))
        .build()
}

fn create_task(agent: &ureq::Agent, key: &str, model: &str, input: serde_json::Value) -> Result<String, String> {
    let body = serde_json::json!({ "model": model, "input": input });
    let resp = agent
        .post(&format!("{API_BASE}/createTask"))
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .send_json(body);
    let v: serde_json::Value = match resp {
        Ok(r) => r.into_json().map_err(|e| format!("bad createTask response: {e}"))?,
        Err(ureq::Error::Status(code, r)) => {
            let detail = r
                .into_json::<serde_json::Value>()
                .ok()
                .and_then(|v| v["msg"].as_str().map(str::to_string))
                .unwrap_or_default();
            return Err(format!("kie.ai error {code}: {detail}"));
        }
        Err(e) => return Err(format!("network error: {e}")),
    };
    if v["code"].as_i64() != Some(200) {
        return Err(format!("kie.ai: {}", v["msg"].as_str().unwrap_or("createTask failed")));
    }
    v["data"]["taskId"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "kie.ai: no taskId in response".to_string())
}

fn poll_task(agent: &ureq::Agent, key: &str, task_id: &str) -> Result<PollState, String> {
    let url = format!("{API_BASE}/recordInfo?taskId={}", enc(task_id));
    let resp = agent.get(&url).set("Authorization", &format!("Bearer {key}")).call();
    let v: serde_json::Value = match resp {
        Ok(r) => r.into_json().map_err(|e| format!("bad recordInfo response: {e}"))?,
        Err(ureq::Error::Status(code, _)) => return Err(format!("kie.ai poll error {code}")),
        Err(e) => return Err(format!("network error: {e}")),
    };
    let data = &v["data"];
    let state = data["state"].as_str().unwrap_or("");
    match state {
        "success" => {
            let raw = data["resultJson"].as_str().unwrap_or("{}");
            let parsed: serde_json::Value = serde_json::from_str(raw).unwrap_or_default();
            let urls: Vec<String> = parsed["resultUrls"]
                .as_array()
                .map(|a| a.iter().filter_map(|u| u.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if urls.is_empty() {
                return Err("kie.ai: task succeeded but returned no result URL".into());
            }
            Ok(PollState::Done(urls))
        }
        "fail" => {
            let msg = data["failMsg"].as_str().filter(|s| !s.is_empty()).unwrap_or("generation failed");
            Err(format!("kie.ai: {msg}"))
        }
        other => Ok(PollState::Running(if other.is_empty() { "queued".into() } else { other.to_string() })),
    }
}

enum PollState {
    Running(String),
    Done(Vec<String>),
}

fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn generate(
    key: &str,
    model: &Model,
    req: &Request,
    mut on_status: impl FnMut(&str),
) -> Result<Vec<String>, String> {
    if key.trim().is_empty() {
        return Err("no kie.ai API key - set it in Settings → AI first".into());
    }
    let agent = api_agent();
    on_status("submitting");
    let task_id = create_task(&agent, key, model.id, build_input(model, req))?;

    let mut last = String::new();
    for _ in 0..300 {
        std::thread::sleep(Duration::from_secs(3));
        match poll_task(&agent, key, &task_id)? {
            PollState::Done(urls) => {
                on_status("success");
                return Ok(urls);
            }
            PollState::Running(state) => {
                if state != last {
                    on_status(&state);
                    last = state;
                }
            }
        }
    }
    Err("kie.ai: timed out waiting for the result".into())
}

fn download(url: &str, dest: &Path) -> Result<(), String> {
    let resp = dl_agent().get(url).call().map_err(|e| format!("download failed: {e}"))?;
    let mut bytes = Vec::new();
    use std::io::Read;
    resp.into_reader().read_to_end(&mut bytes).map_err(|e| format!("download read failed: {e}"))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("can't create output dir: {e}"))?;
    }
    std::fs::write(dest, &bytes).map_err(|e| format!("write failed: {e}"))
}

fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                out.push(l);
            }
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.trim_end_matches('-').chars().count() >= max {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "out".into() } else { out }
}

fn next_output_path(out_dir: &Path, prompt: &str, model: &Model) -> PathBuf {
    let stem = format!("{}-{}", slug(prompt, 40), slug(model.label, 20));
    for n in 1..1000 {
        let candidate = out_dir.join(format!("{stem}-{n:02}.{}", model.ext));
        if !candidate.exists() {
            return candidate;
        }
    }
    out_dir.join(format!("{stem}.{}", model.ext))
}

fn key_path(cfg: &Path) -> PathBuf {
    cfg.join("kie.key")
}

pub fn load_key(cfg: &Path) -> String {
    crate::secret::load_secret(&key_path(cfg))
}

pub fn has_key(cfg: &Path) -> bool {
    std::fs::metadata(key_path(cfg)).map(|m| m.len() > 0).unwrap_or(false)
}

pub fn save_key(cfg: &Path, key: &str) {
    crate::secret::save_secret(&key_path(cfg), key);
}

pub const DEFAULT_CAP: u32 = 50;

fn cap_path(cfg: &Path) -> PathBuf {
    cfg.join("genai_cap.txt")
}

fn usage_path(cfg: &Path) -> PathBuf {
    cfg.join("genai_usage.tsv")
}

pub fn load_cap(cfg: &Path) -> u32 {
    std::fs::read_to_string(cap_path(cfg))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(DEFAULT_CAP)
}

pub fn save_cap(cfg: &Path, cap: u32) {
    let _ = std::fs::write(cap_path(cfg), cap.to_string());
}

fn load_usage(cfg: &Path) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    if let Ok(s) = std::fs::read_to_string(usage_path(cfg)) {
        for line in s.lines() {
            if let Some((p, c)) = line.rsplit_once('\t') {
                out.push((p.to_string(), c.trim().parse().unwrap_or(0)));
            }
        }
    }
    out
}

pub fn usage_for(cfg: &Path, project: &str) -> u32 {
    let key = project_key(project);
    load_usage(cfg).into_iter().find(|(p, _)| *p == key).map(|(_, c)| c).unwrap_or(0)
}

pub fn reset_usage(cfg: &Path, project: &str) {
    let key = project_key(project);
    let rows: Vec<(String, u32)> =
        load_usage(cfg).into_iter().filter(|(p, _)| *p != key).collect();
    let body: String = rows.iter().map(|(p, c)| format!("{p}\t{c}\n")).collect();
    let _ = std::fs::write(usage_path(cfg), body);
}

fn bump_usage(cfg: &Path, project: &str) {
    let key = project_key(project);
    let mut rows = load_usage(cfg);
    match rows.iter_mut().find(|(p, _)| *p == key) {
        Some((_, c)) => *c += 1,
        None => rows.push((key, 1)),
    }
    let body: String = rows.iter().map(|(p, c)| format!("{p}\t{c}\n")).collect();
    let _ = std::fs::write(usage_path(cfg), body);
}

fn project_key(project: &str) -> String {
    if project.trim().is_empty() {
        return "(no project)".into();
    }
    std::fs::canonicalize(project)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| project.to_string())
        .replace('\t', " ")
}

pub fn run(
    cfg: &Path,
    out_dir: &Path,
    project: &str,
    req: &Request,
    on_status: impl FnMut(&str),
) -> Result<Saved, String> {
    let model = MODELS.get(req.model_idx).ok_or("unknown model")?;

    let cap = load_cap(cfg);
    let used = usage_for(cfg, project);
    if used >= cap {
        return Err(format!(
            "project generation cap reached ({used}/{cap}). Raise it in the Mirage tool, or use a different project."
        ));
    }

    let key = load_key(cfg);
    let urls = generate(&key, model, req, on_status)?;
    let url = urls.into_iter().next().ok_or("no result URL")?;
    let dest = next_output_path(out_dir, &req.prompt, model);
    download(&url, &dest)?;
    bump_usage(cfg, project);
    Ok(Saved { path: dest })
}

pub fn cli_main(cfg: &Path, args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--list") {
        print_models();
        return 0;
    }
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return 0;
    }

    let mut model_name = String::new();
    let mut aspect = String::new();
    let mut quality = String::new();
    let mut duration = String::new();
    let mut out = String::from(".");
    let mut project = String::new();
    let mut prompt_parts: Vec<String> = Vec::new();

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--model" | "-m" => model_name = it.next().cloned().unwrap_or_default(),
            "--aspect" | "--ar" => aspect = it.next().cloned().unwrap_or_default(),
            "--res" | "--resolution" | "--mode" | "--quality" => {
                quality = it.next().cloned().unwrap_or_default()
            }
            "--dur" | "--duration" => duration = it.next().cloned().unwrap_or_default(),
            "--out" | "-o" => out = it.next().cloned().unwrap_or_else(|| ".".into()),
            "--project" | "-p" => project = it.next().cloned().unwrap_or_default(),
            "--prompt" => prompt_parts.push(it.next().cloned().unwrap_or_default()),
            other if other.starts_with('-') => {
                eprintln!("unknown flag: {other}");
                return 2;
            }
            other => prompt_parts.push(other.to_string()),
        }
    }

    let prompt = prompt_parts.join(" ").trim().to_string();
    if prompt.is_empty() {
        eprintln!("error: no prompt. Usage: hyperium gen-image --model gpt-image-2 \"a prompt\"");
        return 2;
    }
    let Some(model_idx) = find_model(&model_name) else {
        eprintln!(
            "error: unknown model '{model_name}'. Run `hyperium gen-image --list` to see the choices."
        );
        return 2;
    };
    let model = &MODELS[model_idx];

    let aspect = pick_value("aspect ratio", &aspect, model.aspects, model.aspects.first().copied());
    let aspect = match aspect {
        Ok(v) => v,
        Err(code) => return code,
    };
    let quality = match model.quality.as_ref() {
        Some(k) => match pick_value(k.label, &quality, k.values, k.values.first().copied()) {
            Ok(v) => Some(v),
            Err(code) => return code,
        },
        None => None,
    };
    let duration = match model.duration.as_ref() {
        Some(k) => match pick_value(k.label, &duration, k.values, k.values.first().copied()) {
            Ok(v) => Some(v),
            Err(code) => return code,
        },
        None => None,
    };

    if project.trim().is_empty() {
        project = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    }

    let req = Request { model_idx, prompt, aspect, quality, duration };
    eprintln!("[hyperium] generating with {} ...", model.label);
    match run(cfg, Path::new(&out), &project, &req, |s| eprintln!("[hyperium] {s}")) {
        Ok(saved) => {
            println!("{}", saved.path.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn pick_value(
    label: &str,
    chosen: &str,
    allowed: &[&str],
    default: Option<&str>,
) -> Result<String, i32> {
    if chosen.trim().is_empty() {
        return Ok(default.unwrap_or("").to_string());
    }
    if allowed.iter().any(|v| v.eq_ignore_ascii_case(chosen)) {
        let canon = allowed.iter().find(|v| v.eq_ignore_ascii_case(chosen)).unwrap();
        return Ok((*canon).to_string());
    }
    eprintln!("error: {label} '{chosen}' not allowed. Choices: {}", allowed.join(", "));
    Err(2)
}

fn print_models() {
    println!("Available models (kie.ai):\n");
    for m in MODELS {
        let kind = if m.kind == Kind::Image { "image" } else { "video" };
        println!("  {:<16} [{kind}] {} - {}", short_alias(m), m.provider, m.about);
        println!("      id: {}", m.id);
        println!("      aspect: {}", m.aspects.join(" "));
        if let Some(k) = &m.quality {
            println!("      {}: {}", k.label.to_lowercase(), k.values.join(" "));
        }
        if let Some(k) = &m.duration {
            println!("      duration: {}", k.values.join(" "));
        }
        println!();
    }
}

pub fn short_alias(m: &Model) -> String {
    slug(m.label, 20)
}

fn print_help() {
    println!(
        "hyperium gen-image / gen-video - generate media into a project via kie.ai.\n\
\n\
USAGE:\n\
  hyperium gen-image --model <name> [--aspect R] [--res V] --out DIR \"a prompt\"\n\
  hyperium gen-video --model <name> [--aspect R] [--res V] [--dur N] --out DIR \"a prompt\"\n\
\n\
FLAGS:\n\
  -m, --model <name>   model alias or id (see --list)\n\
      --aspect <R>     aspect ratio, e.g. 16:9 (per-model; defaults to the first allowed)\n\
      --res <V>        resolution / mode, e.g. 2K / 720p / pro (per-model)\n\
      --dur <N>        video duration in seconds (video models only)\n\
  -o, --out <DIR>      output folder (default: current dir)\n\
  -p, --project <DIR>  project used for the generation cap (default: current dir)\n\
      --list           list models and their allowed values\n\
\n\
On success the saved file path is printed to stdout. A per-project cap applies."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_lookup_is_fuzzy() {
        assert_eq!(find_model("gpt-image-2"), Some(0));
        assert_eq!(find_model("nano"), Some(1));
        assert_eq!(find_model("flux"), Some(2));
        assert!(find_model("kling").is_some());
        assert!(find_model("seedance").is_some());
        assert!(find_model("nope-nope").is_none());
    }

    #[test]
    fn slugs_are_tidy() {
        assert_eq!(slug("A cinematic night city!", 40), "a-cinematic-night-city");
        assert_eq!(slug("GPT Image 2", 20), "gpt-image-2");
        assert_eq!(slug("", 10), "out");
        assert_eq!(slug("   ---   ", 10), "out");
    }

    #[test]
    fn input_carries_model_specific_fields() {
        let sd = find_model("seedance").unwrap();
        let req = Request {
            model_idx: sd,
            prompt: "a beach".into(),
            aspect: "16:9".into(),
            quality: Some("720p".into()),
            duration: Some("5".into()),
        };
        let input = build_input(&MODELS[sd], &req);
        assert_eq!(input["duration"], serde_json::json!(5));
        assert_eq!(input["generate_audio"], serde_json::json!(true));
        assert_eq!(input["resolution"], serde_json::json!("720p"));

        let kl = find_model("kling").unwrap();
        let req = Request {
            model_idx: kl,
            prompt: "a stage".into(),
            aspect: "16:9".into(),
            quality: Some("pro".into()),
            duration: Some("5".into()),
        };
        let input = build_input(&MODELS[kl], &req);
        assert_eq!(input["duration"], serde_json::json!("5"));
        assert_eq!(input["mode"], serde_json::json!("pro"));
        assert_eq!(input["multi_prompt"], serde_json::json!([]));
    }
}
