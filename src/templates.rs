use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::sync;

#[derive(Clone, Default)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub short: String,
    pub long: String,
    pub category: String,
    pub tags: Vec<String>,
    pub has_thumb: bool,
}

#[derive(Default)]
pub struct Shared {
    pub busy: bool,
    pub loaded: bool,
    pub message: String,
    pub entries: Vec<Entry>,
    pub to_type: Option<String>,
}

pub fn cache_dir(cfg: &Path) -> PathBuf {
    cfg.join("templates-cache")
}

pub fn thumb_path(cfg: &Path, id: &str) -> PathBuf {
    cache_dir(cfg).join(format!("{id}.img"))
}

pub fn categories(entries: &[Entry]) -> Vec<String> {
    let mut v: Vec<String> =
        entries.iter().map(|e| e.category.clone()).filter(|c| !c.is_empty()).collect();
    v.sort();
    v.dedup();
    v
}

pub fn matches(e: &Entry, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    e.title.to_lowercase().contains(&q)
        || e.short.to_lowercase().contains(&q)
        || e.category.to_lowercase().contains(&q)
        || e.tags.iter().any(|t| t.to_lowercase().contains(&q))
}

pub fn guide_filename(e: &Entry) -> String {
    format!("{}.md", e.id)
}

pub fn parse_catalog(v: &serde_json::Value) -> Vec<Entry> {
    let mut out = Vec::new();
    let Some(arr) = v["templates"].as_array() else {
        return out;
    };
    for t in arr {
        let id = t["id"].as_str().unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        out.push(Entry {
            title: t["title"].as_str().unwrap_or(&id).to_string(),
            short: t["short"].as_str().unwrap_or("").to_string(),
            long: t["long"].as_str().unwrap_or("").to_string(),
            category: t["category"].as_str().unwrap_or("").to_string(),
            tags: t["tags"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            has_thumb: t["has_thumb"].as_bool().unwrap_or(false),
            id,
        });
    }
    out
}

pub fn refresh(shared: Arc<Mutex<Shared>>, cfg: PathBuf) {
    {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        if s.busy {
            return;
        }
        s.busy = true;
        s.message = "Loading templates...".into();
    }
    match sync::templates_catalog(&cfg).map(|v| parse_catalog(&v)) {
        Ok(entries) => {
            for e in &entries {
                if !e.has_thumb {
                    continue;
                }
                let dest = thumb_path(&cfg, &e.id);
                if !dest.exists() {
                    let _ = sync::download_template_thumb(&cfg, &e.id, &dest);
                }
            }
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.entries = entries;
            s.loaded = true;
            s.busy = false;
            s.message.clear();
        }
        Err(err) => {
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.busy = false;
            s.message = err;
        }
    }
}

pub fn use_template(
    shared: Arc<Mutex<Shared>>,
    cfg: PathBuf,
    project_dir: PathBuf,
    entry: Entry,
) {
    {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        s.busy = true;
        s.message = format!("Downloading {}...", entry.title);
    }
    let outcome = sync::template_download(&cfg, &entry.id).and_then(|bytes| {
        let filename = guide_filename(&entry);
        std::fs::write(project_dir.join(&filename), &bytes)
            .map(|_| filename)
            .map_err(|e| format!("write failed: {e}"))
    });
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
    s.busy = false;
    match outcome {
        Ok(filename) => {
            s.message.clear();
            s.to_type = Some(format!("read {filename}"));
        }
        Err(err) => s.message = err,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_filter() {
        let json = serde_json::json!({
            "templates": [
                { "id": "php-mvc", "title": "PHP MVC", "short": "Vanilla PHP MVC",
                  "long": "A clean MVC skeleton", "category": "Backend",
                  "tags": ["php", "mvc"], "has_thumb": true },
                { "id": "", "title": "dropped: empty id" },
                { "id": "html-site", "title": "HTML site", "category": "Frontend",
                  "has_thumb": false }
            ]
        });
        let entries = parse_catalog(&json);
        assert_eq!(entries.len(), 2, "the empty-id entry is skipped");
        assert_eq!(entries[0].id, "php-mvc");
        assert!(entries[0].has_thumb);
        assert_eq!(entries[0].tags, vec!["php", "mvc"]);
        assert_eq!(entries[1].short, "");
        assert!(!entries[1].has_thumb);

        assert!(matches(&entries[0], "mvc"));
        assert!(matches(&entries[0], "BACKEND"));
        assert!(matches(&entries[0], ""));
        assert!(!matches(&entries[1], "php"));

        assert_eq!(categories(&entries), vec!["Backend", "Frontend"]);
        assert_eq!(guide_filename(&entries[0]), "php-mvc.md");
    }
}
