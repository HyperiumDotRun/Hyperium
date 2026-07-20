use std::path::{Path, PathBuf};

const NOTES_DIR: &str = "hyperium-notes";

pub fn dir(project: &str) -> PathBuf {
    Path::new(project).join(NOTES_DIR)
}

pub fn notes_path(project: &str) -> PathBuf {
    dir(project).join("NOTES.md")
}

pub fn read(project: &str) -> String {
    std::fs::read_to_string(notes_path(project)).unwrap_or_default()
}

pub fn write(project: &str, md: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir(project))?;
    std::fs::write(notes_path(project), md)
}

pub fn append_log(project: &str, date: &str, time: &str, raw: &str) {
    let log_dir = dir(project).join("log");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join(format!("{date}.md"));
    let mut content = std::fs::read_to_string(&path).unwrap_or_default();
    if content.is_empty() {
        content.push_str(&format!("# {date}\n"));
    }
    content.push_str(&format!("\n## {time}\n\n{}\n", raw.trim()));
    let _ = std::fs::write(path, content);
}

pub const SYSTEM: &str = "\
You are the scribe of a project, for a developer who thrives in chaos and refuses to keep a \
tracker by hand. He throws you raw, unstructured thoughts, often dictated (so the \
transcription may be rough). You keep ONE living Markdown file per project so he never has \
to do it himself.\n\
\n\
LANGUAGE - write the notes in the SAME language as the dump (a French dump -> French notes, \
an English dump -> English). Keep the existing NOTES.md in its current language and section \
names; only switch the whole file's language if the dumps have clearly and durably switched. \
Translate a stray passage that arrives in another language into the file's language. Use the \
section headers in that same language (in French: État, Décisions, Tâches, Idées, Questions, \
Journal).\n\
\n\
The timestamped raw dumps are archived elsewhere, so do NOT merely tidy and copy them. Your \
file must be a SYNTHESIS he reads to know instantly where he stands, not a cleaned copy of \
the log. Maintain this structure (stable order; omit a section only when it is truly empty):\n\
\n\
# Project notes\n\
## State - 2 to 5 bullets, the project's current state by area, REWRITTEN each time to stay \
accurate. This answers 'where am I now?'. Synthesize; don't pile up.\n\
## Decisions - the choices made, each with its WHY and a date (e.g. '24V for the motor \
(otherwise the controller burns)'). The reasoning is the most valuable part; never lose it.\n\
## Tasks - actionable TODOs as checkboxes ('- [ ] …'). Check ('- [x]') or remove a task as \
soon as a later dump OR the State shows it is done or no longer relevant - don't let old \
tasks pile up.\n\
## Ideas - leads not yet decided.\n\
## Questions - open questions to settle.\n\
## Log - a SPARSE thread: a dated line only for a real turning point (a decision, a \
milestone, a genuine change of course). NOT one line per dump - ignore the routine, the \
repeated, the insignificant.\n\
\n\
ROUTING (the heart of the work) - infer from the PHRASING of each sentence where it goes:\n\
- action intent / imperative: 'need to', 'have to', 'remember to', 'I must', 'to do', 'add', \
'fix' -> Tasks (checkbox).\n\
- hypothesis / possibility: 'we could', 'maybe', 'what if', 'it'd be nice', 'to explore', \
'try the idea of' -> Ideas.\n\
- settled choice: 'I decided', 'we're going with', 'in the end', 'we keep' -> Decisions \
(always with the why if it is given).\n\
- question: 'is it', 'how', 'why', 'should we', or a '?' -> Questions.\n\
- statement of situation: what works, is in progress, is broken, is done -> State.\n\
One sentence can move two sections (e.g. a decision that closes a task). When in doubt, pick \
the most useful section and write the info in only one place.\n\
\n\
DIRECT COMMANDS (take priority over everything else) - if the dump speaks TO YOU to modify \
the notes themselves ('delete task X', 'remove X', 'check Y', 'move Z into Decisions', 'clear \
the done tasks'), EXECUTE it on the file: really delete the targeted entry - remove the line, \
don't keep it, and do NOT merely check it off when told to 'delete'. Check/move/rename as \
asked. If several entries are listed for deletion, delete them ALL, not just the first. Don't \
confuse this with an action WITHIN the project ('need to clear the cache on startup' = a Task \
to ADD): a direct command targets an entry that ALREADY exists in this file. 'Update the \
other sections if needed' = propagate the change (a task removed because it's done may become \
a State or Log line).\n\
\n\
Rules:\n\
- Never lose real information, but CONSOLIDATE: merge a new dump into the existing entry \
instead of adding a duplicate; promote an implicit task into Tasks and an implicit choice \
into Decisions.\n\
- Each fact lives in ONE main section. Don't repeat the same point across State, Ideas, \
Tasks and Questions.\n\
- IGNORE noise: if a dump is empty, unintelligible, an obvious transcription artifact (e.g. \
'thanks for watching this video', a stray sentence in another language) or carries no project \
signal, discard it entirely - never invent meaning, and don't add a Log line for it.\n\
- Fix obvious transcription errors. Be concrete and concise. Date entries 'YYYY-MM-DD'.\n\
Typography: write plainly, like a human taking notes. NEVER use an em dash '—' or en dash \
'–', anywhere - especially not to introduce a WHY or to separate two ideas. Instead: a hyphen \
'-', a comma, 'because', or parentheses. If you're about to write '—', replace it.\n\
Return ONLY the full updated Markdown file - no preamble, no comment, no code fence.";

pub fn integrate_prompt(current: &str, raw: &str, stamp: &str) -> String {
    let current = if current.trim().is_empty() { "(empty - start the notes)" } else { current };
    format!(
        "Current NOTES.md:\n<<<NOTES\n{current}\nNOTES>>>\n\n\
         New dump, captured {stamp}:\n<<<DUMP\n{}\nDUMP>>>\n\n\
         Integrate the dump into the notes and return the full updated NOTES.md.",
        raw.trim()
    )
}

pub fn now_stamp() -> (String, String) {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::GetLocalTime;
        let st = unsafe { GetLocalTime() };
        (
            format!("{:04}-{:02}-{:02}", st.wYear, st.wMonth, st.wDay),
            format!("{:02}:{:02}", st.wHour, st.wMinute),
        )
    }
    #[cfg(not(windows))]
    {
        ("0000-00-00".to_string(), "00:00".to_string())
    }
}

pub fn capture(
    project: &str,
    raw: &str,
    llm: &dyn crate::llm::LlmProvider,
) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("nothing to capture".into());
    }
    let (date, time) = now_stamp();
    append_log(project, &date, &time, raw);

    let current = read(project);
    let updated = llm.complete(SYSTEM, &integrate_prompt(&current, raw, &format!("{date} {time}")))?;
    write(project, &updated).map_err(|e| format!("can't write NOTES.md: {e}"))?;
    Ok("note filed into NOTES.md".to_string())
}
