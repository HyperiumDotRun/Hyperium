use std::cell::RefCell;
use std::io::{Read, Write};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

struct Size {
    cols: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone)]
pub struct Proxy {
    replies: Rc<RefCell<Vec<u8>>>,
    title: Rc<RefCell<Option<String>>>,
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                self.replies.borrow_mut().extend_from_slice(text.as_bytes());
            }
            Event::Title(title) => *self.title.borrow_mut() = Some(title),
            Event::ResetTitle => *self.title.borrow_mut() = None,
            _ => {}
        }
    }
}

pub struct PtySession {
    term: Term<Proxy>,
    parser: Processor,
    proxy: Proxy,
    rx: Receiver<Vec<u8>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    pid: Option<u32>,
    job: Option<isize>,
    cols: usize,
    lines: usize,
    sel_anchor: Option<Point>,
}

impl PtySession {
    pub fn spawn(
        cwd: &str,
        cols: usize,
        lines: usize,
        on_output: std::sync::Arc<dyn Fn() + Send + Sync>,
    ) -> Option<Self> {
        let cols = cols.max(1);
        let lines = lines.max(1);

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: lines as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .ok()?;

        let mut cmd = CommandBuilder::new("powershell.exe");
        if std::path::Path::new(cwd).is_dir() {
            cmd.cwd(cwd);
        }
        let child = pair.slave.spawn_command(cmd).ok()?;
        let pid = child.process_id();
        drop(pair.slave);

        #[cfg(windows)]
        let job = pid.and_then(assign_to_job);
        #[cfg(not(windows))]
        let job: Option<isize> = None;

        let reader = pair.master.try_clone_reader().ok()?;
        let writer = pair.master.take_writer().ok()?;

        let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        on_output();
                    }
                }
            }
        });

        let proxy = Proxy {
            replies: Rc::new(RefCell::new(Vec::new())),
            title: Rc::new(RefCell::new(None)),
        };
        let term = Term::new(Config::default(), &Size { cols, lines }, proxy.clone());

        Some(Self {
            term,
            parser: Processor::new(),
            proxy,
            rx,
            writer,
            master: pair.master,
            child,
            pid,
            job,
            cols,
            lines,
            sel_anchor: None,
        })
    }

    pub fn title(&self) -> Option<String> {
        self.proxy.title.borrow().clone()
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn scroll_wheel(&mut self, lines: i32, col: usize, row: usize, shift: bool) -> bool {
        if lines == 0 {
            return false;
        }
        let mode = *self.term.mode();
        let up = lines > 0;
        let count = lines.unsigned_abs();

        if !shift && mode.intersects(TermMode::MOUSE_MODE) {
            let btn = if up { 64 } else { 65 };
            let cx = col + 1;
            let cy = row + 1;
            let mut seq = Vec::new();
            for _ in 0..count {
                if mode.contains(TermMode::SGR_MOUSE) {
                    seq.extend_from_slice(format!("\x1b[<{btn};{cx};{cy}M").as_bytes());
                } else {
                    let b = (btn + 32) as u8;
                    let x = (cx.min(223) as u8).saturating_add(32);
                    let y = (cy.min(223) as u8).saturating_add(32);
                    seq.extend_from_slice(&[0x1b, b'[', b'M', b, x, y]);
                }
            }
            self.send_input(&seq);
            return false;
        }

        if !shift
            && mode.contains(TermMode::ALT_SCREEN)
            && mode.contains(TermMode::ALTERNATE_SCROLL)
        {
            let arrow: &[u8] = match (mode.contains(TermMode::APP_CURSOR), up) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1bOB",
                (false, true) => b"\x1b[A",
                (false, false) => b"\x1b[B",
            };
            let mut seq = Vec::new();
            for _ in 0..count {
                seq.extend_from_slice(arrow);
            }
            self.send_input(&seq);
            return false;
        }

        self.term.scroll_display(Scroll::Delta(lines));
        true
    }

    fn screen_point(&self, col: usize, row: usize) -> Point {
        let offset = self.term.grid().display_offset() as i32;
        Point::new(
            Line(row as i32 - offset),
            Column(col.min(self.cols.saturating_sub(1))),
        )
    }

    pub fn selection_start(&mut self, col: usize, row: usize) {
        let anchor = self.screen_point(col, row);
        self.sel_anchor = Some(anchor);
        self.set_selection(anchor, anchor);
    }

    pub fn selection_update(&mut self, col: usize, row: usize) {
        if let Some(anchor) = self.sel_anchor {
            let end = self.screen_point(col, row);
            self.set_selection(anchor, end);
        }
    }

    fn set_selection(&mut self, anchor: Point, end: Point) {
        let (a_side, e_side) = if end >= anchor {
            (Side::Left, Side::Right)
        } else {
            (Side::Right, Side::Left)
        };
        let mut sel = Selection::new(SelectionType::Simple, anchor, a_side);
        sel.update(end, e_side);
        self.term.selection = Some(sel);
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
        self.sel_anchor = None;
    }

    pub fn pump(&mut self) -> bool {
        let mut dirty = false;
        while let Ok(chunk) = self.rx.try_recv() {
            self.parser.advance(&mut self.term, &chunk);
            dirty = true;
        }
        if dirty {
            let replies = std::mem::take(&mut *self.proxy.replies.borrow_mut());
            if !replies.is_empty() {
                let _ = self.writer.write_all(&replies);
                let _ = self.writer.flush();
            }
        }
        dirty
    }

    pub fn send_input(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            let _ = self.writer.write_all(bytes);
            let _ = self.writer.flush();
        }
    }

    pub fn resize(&mut self, cols: usize, lines: usize) {
        let cols = cols.max(1);
        let lines = lines.max(1);
        if cols == self.cols && lines == self.lines {
            return;
        }
        self.cols = cols;
        self.lines = lines;
        let _ = self.master.resize(PtySize {
            rows: lines as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.term.resize(Size { cols, lines });
    }

    pub fn term(&self) -> &Term<Proxy> {
        &self.term
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if let Some(h) = self.job.take() {
                close_job(h);
            } else if let Some(pid) = self.pid {
                kill_tree(pid);
            }
        }
        let _ = self.child.kill();
    }
}

#[cfg(windows)]
pub(crate) fn assign_to_job(pid: u32) -> Option<isize> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .is_ok();
        if !ok {
            let _ = CloseHandle(job);
            return None;
        }
        let Ok(proc) = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) else {
            let _ = CloseHandle(job);
            return None;
        };
        let assigned = AssignProcessToJobObject(job, proc).is_ok();
        let _ = CloseHandle(proc);
        if assigned {
            Some(job.0 as isize)
        } else {
            let _ = CloseHandle(job);
            None
        }
    }
}

#[cfg(windows)]
pub(crate) fn close_job(handle: isize) {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    let _ = unsafe { CloseHandle(HANDLE(handle as *mut core::ffi::c_void)) };
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
