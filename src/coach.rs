use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Habit {
    pub id: String,
    pub label: String,
    pub noun: String,
    pub verb: String,
    pub icon: String,
    pub daily_target: u32,
    pub min_chunk: u32,
    pub max_chunk: u32,
}

impl Habit {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: &str,
        label: &str,
        noun: &str,
        verb: &str,
        icon: &str,
        daily_target: u32,
        min_chunk: u32,
        max_chunk: u32,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            noun: noun.into(),
            verb: verb.into(),
            icon: icon.into(),
            daily_target,
            min_chunk,
            max_chunk,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Progress {
    pub done: u32,
    pub streak: u32,
}

#[derive(Clone)]
pub struct Nudge {
    pub habit_id: String,
    pub icon: String,
    pub verb: String,
    pub noun: String,
    pub amount: u32,
}

impl Nudge {
    pub fn message(&self) -> String {
        format!("{} {} {} {}", self.icon, self.verb, self.amount, self.noun)
    }
}

pub struct Coach {
    pub habits: Vec<Habit>,
    progress: HashMap<String, Progress>,
    pub interval_min_secs: u64,
    pub interval_max_secs: u64,
    rng: u64,
    today: String,
    last_done: HashMap<String, String>,
}

impl Coach {
    pub fn load(config_dir: &Path) -> Self {
        let habits = load_habits(&habits_path(config_dir)).unwrap_or_else(default_habits);
        let (mut progress, last_done) = load_state(&state_path(config_dir));
        let today = local_date();
        let td = load_today(&history_path(config_dir), &today);
        for (id, p) in progress.iter_mut() {
            p.done = td.get(id).copied().unwrap_or(0);
        }
        let mut coach = Coach {
            habits,
            progress,
            interval_min_secs: 20 * 60,
            interval_max_secs: 40 * 60,
            rng: seed(),
            today,
            last_done,
        };
        coach.refresh_streaks();
        coach
    }

    pub fn save_state(&self, config_dir: &Path) {
        save_state(&state_path(config_dir), &self.habits, &self.progress, &self.last_done);
    }

    pub fn save_history(&self, config_dir: &Path) {
        save_history(&history_path(config_dir), &self.today, &self.habits, &self.progress);
    }

    fn refresh_streaks(&mut self) {
        let today = self.today.clone();
        let yesterday = prev_date(&today);
        for (id, p) in self.progress.iter_mut() {
            let alive = matches!(self.last_done.get(id), Some(d) if *d == today || *d == yesterday);
            if !alive {
                p.streak = 0;
            }
        }
    }

    pub fn reset_streaks(&mut self) {
        for p in self.progress.values_mut() {
            p.streak = 0;
        }
    }

    fn roll_day(&mut self) {
        let now = local_date();
        if now != self.today {
            self.today = now;
            for p in self.progress.values_mut() {
                p.done = 0;
            }
            self.refresh_streaks();
        }
    }

    pub fn save_habits(&self, config_dir: &Path) {
        save_habits(&habits_path(config_dir), &self.habits);
    }

    pub fn progress_of(&self, id: &str) -> Progress {
        self.progress.get(id).copied().unwrap_or_default()
    }

    pub fn remaining(&self, habit: &Habit) -> u32 {
        habit.daily_target.saturating_sub(self.progress_of(&habit.id).done)
    }

    pub fn next_nudge(&mut self) -> Option<Nudge> {
        self.roll_day();
        self.next_nudge_now()
    }

    fn next_nudge_now(&mut self) -> Option<Nudge> {
        let candidates: Vec<usize> = (0..self.habits.len())
            .filter(|&i| self.remaining(&self.habits[i]) > 0)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let pick = candidates[self.rand_below(candidates.len() as u64) as usize];
        let h = self.habits[pick].clone();
        let rem = self.remaining(&h);
        let hi = h.max_chunk.min(rem).max(1);
        let lo = h.min_chunk.min(hi);
        let amount = self.rand_range(lo, hi);
        Some(Nudge {
            habit_id: h.id,
            icon: h.icon,
            verb: h.verb,
            noun: h.noun,
            amount,
        })
    }

    pub fn mark_done(&mut self, habit_id: &str, amount: u32) -> bool {
        self.roll_day();
        self.record(habit_id, amount)
    }

    fn record(&mut self, habit_id: &str, amount: u32) -> bool {
        let Some(habit) = self.habits.iter().find(|h| h.id == habit_id) else {
            return false;
        };
        let target = habit.daily_target;
        let entry = self.progress.entry(habit_id.to_string()).or_default();
        let was_complete = entry.done >= target;
        entry.done = (entry.done + amount).min(target);
        let now_complete = entry.done >= target;

        if !now_complete || was_complete {
            return false;
        }
        let continues = self.last_done.get(habit_id).map(|d| d == &prev_date(&self.today)).unwrap_or(false);
        let entry = self.progress.entry(habit_id.to_string()).or_default();
        entry.streak = if continues { entry.streak + 1 } else { 1 };
        self.last_done.insert(habit_id.to_string(), self.today.clone());
        true
    }

    pub fn next_interval_secs(&mut self) -> u64 {
        self.rand_range(self.interval_min_secs.max(1) as u32, self.interval_max_secs.max(1) as u32)
            as u64
    }

    fn rand_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn rand_below(&mut self, n: u64) -> u64 {
        if n <= 1 { 0 } else { self.rand_u64() % n }
    }

    fn rand_range(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            lo
        } else {
            lo + (self.rand_below((hi - lo + 1) as u64) as u32)
        }
    }
}

pub fn default_habits() -> Vec<Habit> {
    vec![
        Habit::new("pushups", "Push-ups", "push-ups", "Do", "💪", 40, 3, 8),
        Habit::new("water", "Water", "glasses", "Drink", "💧", 8, 1, 2),
    ]
}

fn habits_path(config_dir: &Path) -> PathBuf {
    config_dir.join("habits.tsv")
}

fn state_path(config_dir: &Path) -> PathBuf {
    config_dir.join("coach_state.tsv")
}

fn load_habits(path: &Path) -> Option<Vec<Habit>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut habits = Vec::new();
    for line in content.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 8 {
            continue;
        }
        let (Ok(daily_target), Ok(min_chunk), Ok(max_chunk)) =
            (f[5].parse::<u32>(), f[6].parse::<u32>(), f[7].parse::<u32>())
        else {
            continue;
        };
        if f[0].is_empty() {
            continue;
        }
        habits.push(Habit {
            id: f[0].into(),
            label: f[1].into(),
            noun: f[2].into(),
            verb: f[3].into(),
            icon: f[4].into(),
            daily_target,
            min_chunk,
            max_chunk,
        });
    }
    (!habits.is_empty()).then_some(habits)
}

fn save_habits(path: &Path, habits: &[Habit]) {
    let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    let mut out = String::new();
    for h in habits {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            clean(&h.id),
            clean(&h.label),
            clean(&h.noun),
            clean(&h.verb),
            clean(&h.icon),
            h.daily_target,
            h.min_chunk,
            h.max_chunk,
        ));
    }
    let _ = std::fs::write(path, out);
}

fn load_state(path: &Path) -> (HashMap<String, Progress>, HashMap<String, String>) {
    let mut progress = HashMap::new();
    let mut last_done = HashMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return (progress, last_done);
    };
    for line in content.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 3 || f[0].is_empty() {
            continue;
        }
        let (Ok(done), Ok(streak)) = (f[1].parse::<u32>(), f[2].parse::<u32>()) else {
            continue;
        };
        progress.insert(f[0].to_string(), Progress { done, streak });
        if let Some(d) = f.get(3)
            && !d.is_empty()
        {
            last_done.insert(f[0].to_string(), d.to_string());
        }
    }
    (progress, last_done)
}

fn save_state(
    path: &Path,
    habits: &[Habit],
    progress: &HashMap<String, Progress>,
    last_done: &HashMap<String, String>,
) {
    let mut out = String::new();
    for h in habits {
        let p = progress.get(&h.id).copied().unwrap_or_default();
        let ld = last_done.get(&h.id).map(String::as_str).unwrap_or("");
        out.push_str(&format!("{}\t{}\t{}\t{}\n", h.id, p.done, p.streak, ld));
    }
    let _ = std::fs::write(path, out);
}

fn history_path(config_dir: &Path) -> PathBuf {
    config_dir.join("coach_history.tsv")
}

fn load_today(path: &Path, today: &str) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return map;
    };
    for line in content.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 4 || f[0] != today || f[1].is_empty() {
            continue;
        }
        if let Ok(done) = f[3].parse::<u32>() {
            map.insert(f[1].to_string(), done);
        }
    }
    map
}

fn save_history(
    path: &Path,
    today: &str,
    habits: &[Habit],
    progress: &HashMap<String, Progress>,
) {
    let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    let mut kept = String::new();
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let date = line.split('\t').next().unwrap_or("");
            if date == today {
                continue;
            }
            kept.push_str(line);
            kept.push('\n');
        }
    }
    let mut out = String::from("# date\thabit_id\tlabel\tdone\ttarget\tstreak\n");
    out.push_str(&kept);
    for h in habits {
        let done = progress.get(&h.id).map(|p| p.done).unwrap_or(0);
        let streak = progress.get(&h.id).map(|p| p.streak).unwrap_or(0);
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            today,
            clean(&h.id),
            clean(&h.label),
            done,
            h.daily_target,
            streak,
        ));
    }
    let _ = std::fs::write(path, out);
}

pub fn history_scores(config_dir: &Path) -> HashMap<String, f32> {
    let mut sums: HashMap<String, (f32, u32)> = HashMap::new();
    if let Ok(content) = std::fs::read_to_string(history_path(config_dir)) {
        for line in content.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 5 || f[0].is_empty() {
                continue;
            }
            let (Ok(done), Ok(target)) = (f[3].parse::<f32>(), f[4].parse::<f32>()) else {
                continue;
            };
            let ratio = if target > 0.0 { (done / target).min(1.0) } else { 0.0 };
            let e = sums.entry(f[0].to_string()).or_insert((0.0, 0));
            e.0 += ratio;
            e.1 += 1;
        }
    }
    sums.into_iter().map(|(d, (s, n))| (d, if n > 0 { s / n as f32 } else { 0.0 })).collect()
}

pub fn local_date() -> String {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let st = unsafe { GetLocalTime() };
        format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay)
    }
    #[cfg(not(windows))]
    {
        "0000-00-00".to_string()
    }
}

fn prev_date(date: &str) -> String {
    let Some((y, m, d)) = parse_ymd(date) else { return String::new() };
    let (y2, m2, d2) = civil_from_days(days_from_civil(y, m, d) - 1);
    format!("{y2:04}-{m2:02}-{d2:02}")
}

fn parse_ymd(s: &str) -> Option<(i64, u32, u32)> {
    let mut it = s.split('-');
    let y = it.next()?.parse::<i64>().ok()?;
    let m = it.next()?.parse::<u32>().ok()?;
    let d = it.next()?.parse::<u32>().ok()?;
    Some((y, m, d))
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn seed() -> u64 {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    n | 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coach_on(date: &str, target: u32) -> Coach {
        Coach {
            habits: vec![Habit::new("pushups", "Push-ups", "push-ups", "Do", "💪", target, 3, 8)],
            progress: HashMap::new(),
            interval_min_secs: 60,
            interval_max_secs: 60,
            rng: 1,
            today: date.to_string(),
            last_done: HashMap::new(),
        }
    }

    fn advance_to(c: &mut Coach, date: &str) {
        c.today = date.to_string();
        for p in c.progress.values_mut() {
            p.done = 0;
        }
        c.refresh_streaks();
    }

    #[test]
    fn prev_date_walks_back_across_boundaries() {
        assert_eq!(prev_date("2026-06-16"), "2026-06-15");
        assert_eq!(prev_date("2026-03-01"), "2026-02-28");
        assert_eq!(prev_date("2024-03-01"), "2024-02-29");
        assert_eq!(prev_date("2026-01-01"), "2025-12-31");
        assert_eq!(prev_date("garbage"), "");
    }

    #[test]
    fn daily_target_is_a_cap() {
        let mut c = coach_on("2026-06-16", 10);
        let h = c.habits[0].clone();
        assert!(!c.record("pushups", 4));
        assert_eq!(c.remaining(&h), 6);
        assert!(c.record("pushups", 6));
        assert_eq!(c.remaining(&h), 0);
        assert!(c.next_nudge_now().is_none(), "capped habit must not be nudged again today");
        assert!(!c.record("pushups", 9));
        assert_eq!(c.progress_of("pushups").done, 10);
        assert_eq!(c.progress_of("pushups").streak, 1, "one streak per day max");
    }

    #[test]
    fn streak_grows_on_consecutive_days() {
        let mut c = coach_on("2026-06-14", 5);
        assert!(c.record("pushups", 5));
        assert_eq!(c.progress_of("pushups").streak, 1);
        advance_to(&mut c, "2026-06-15");
        assert!(c.record("pushups", 5));
        assert_eq!(c.progress_of("pushups").streak, 2);
        advance_to(&mut c, "2026-06-16");
        assert!(c.record("pushups", 5));
        assert_eq!(c.progress_of("pushups").streak, 3);
    }

    #[test]
    fn streak_resets_after_a_missed_day() {
        let mut c = coach_on("2026-06-14", 5);
        c.record("pushups", 5);
        assert_eq!(c.progress_of("pushups").streak, 1);
        advance_to(&mut c, "2026-06-16");
        assert_eq!(c.progress_of("pushups").streak, 0, "a missed day breaks the streak on rollover");
        assert!(c.record("pushups", 5));
        assert_eq!(c.progress_of("pushups").streak, 1, "completing restarts the streak at 1");
    }

    #[test]
    fn done_resets_each_day_quota_reopens() {
        let mut c = coach_on("2026-06-15", 5);
        c.record("pushups", 5);
        assert_eq!(c.remaining(&c.habits[0].clone()), 0);
        advance_to(&mut c, "2026-06-16");
        assert_eq!(c.remaining(&c.habits[0].clone()), 5, "fresh quota next day");
        assert!(c.next_nudge().is_some(), "a fresh day reopens nudges");
    }
}
