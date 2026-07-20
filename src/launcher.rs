use std::path::Path;

#[derive(Clone)]
pub enum CommandKind {
    External { path: String, args: String },
    Internal { id: String },
}

#[derive(Clone)]
pub struct Command {
    pub name: String,
    pub kind: CommandKind,
}

impl Command {
    pub fn detail(&self) -> String {
        match &self.kind {
            CommandKind::External { path, .. } => Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone()),
            CommandKind::Internal { .. } => "Hyperium tool".to_string(),
        }
    }

    pub fn launch(&self) -> std::io::Result<()> {
        match &self.kind {
            CommandKind::External { path, args } => {
                let mut cmd = std::process::Command::new(path);
                for a in split_args(args) {
                    cmd.arg(a);
                }
                if let Some(dir) = Path::new(path).parent().filter(|d| d.is_dir()) {
                    cmd.current_dir(dir);
                }
                cmd.spawn().map(|_| ())
            }
            CommandKind::Internal { .. } => Ok(()),
        }
    }
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in s.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn match_commands(commands: &[Command], query: &str) -> Vec<Command> {
    let q = query.trim().to_lowercase();
    if q.len() < 3 {
        return commands.to_vec();
    }
    let mut hits: Vec<Command> = commands
        .iter()
        .filter(|c| c.name.to_lowercase().contains(&q))
        .cloned()
        .collect();
    hits.sort_by_key(|c| {
        let name = c.name.to_lowercase();
        (!name.starts_with(&q), name)
    });
    hits
}

pub fn load_commands(path: &Path) -> Vec<Command> {
    let mut commands = Vec::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return commands;
    };
    for line in content.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        match fields.next() {
            Some("external") => {
                let name = fields.next().unwrap_or("").to_string();
                let exe = fields.next().unwrap_or("").to_string();
                let args = fields.next().unwrap_or("").to_string();
                if !name.is_empty() && !exe.is_empty() {
                    commands.push(Command {
                        name,
                        kind: CommandKind::External { path: exe, args },
                    });
                }
            }
            Some("internal") => {
                let name = fields.next().unwrap_or("").to_string();
                let id = fields.next().unwrap_or("").to_string();
                if !name.is_empty() && !id.is_empty() {
                    commands.push(Command { name, kind: CommandKind::Internal { id } });
                }
            }
            _ => continue,
        }
    }
    commands
}

pub fn save_commands(path: &Path, commands: &[Command]) {
    let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    let mut out = String::new();
    for c in commands {
        match &c.kind {
            CommandKind::External { path, args } => {
                out.push_str("external\t");
                out.push_str(&clean(&c.name));
                out.push('\t');
                out.push_str(&clean(path));
                out.push('\t');
                out.push_str(&clean(args));
                out.push('\n');
            }
            CommandKind::Internal { id } => {
                out.push_str("internal\t");
                out.push_str(&clean(&c.name));
                out.push('\t');
                out.push_str(&clean(id));
                out.push('\n');
            }
        }
    }
    let _ = std::fs::write(path, out);
}
