use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Result;
use chrono::{Local, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use tui_textarea::{CursorMove, TextArea};

use crate::{
    clipboard, format,
    storage::{self, Store, previous_workday},
    vim::{Mode, VimBuffer},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Did,
    Planning,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verb {
    Yank,
}

pub struct App<'a> {
    path: PathBuf,
    store: Store,
    today: NaiveDate,
    yesterday: NaiveDate,
    yesterday_planning: Vec<String>,
    did_buf: VimBuffer<'a>,
    planning_buf: VimBuffer<'a>,
    focus: Pane,
    quit: bool,
    /// `Some` when the user is mid-`:command` entry, contains the typed text so far.
    ex_command: Option<String>,
    /// One-line transient message shown in the status bar (e.g. "written").
    status_msg: Option<String>,
    /// When `status_msg` should be cleared automatically.
    status_msg_until: Option<Instant>,
    /// `Some` when a verb has been pressed and we're waiting for its target key.
    pending_verb: Option<Verb>,
}

const STATUS_MSG_DURATION: Duration = Duration::from_secs(2);

impl<'a> App<'a> {
    pub fn new(path: PathBuf) -> Result<Self> {
        let store = storage::load(&path)?;
        let today = Local::now().date_naive();
        let yesterday = previous_workday(today);

        let yesterday_planning: Vec<String> = store
            .get(yesterday)
            .map(|e| e.planning.clone())
            .unwrap_or_default();

        // Yesterday's `did`: use existing did, else fall back to its planning as a draft prompt.
        let did_lines: Vec<String> = store
            .get(yesterday)
            .map(|e| if e.did.is_empty() { e.planning.clone() } else { e.did.clone() })
            .unwrap_or_default();

        let planning_lines: Vec<String> = store
            .get(today)
            .map(|e| e.planning.clone())
            .unwrap_or_default();

        let did_buf = VimBuffer::new(make_textarea(did_lines));
        let planning_buf = VimBuffer::new(make_textarea(planning_lines));

        let mut app = Self {
            path,
            store,
            today,
            yesterday,
            yesterday_planning,
            did_buf,
            planning_buf,
            focus: Pane::Did,
            quit: false,
            ex_command: None,
            status_msg: None,
            status_msg_until: None,
            pending_verb: None,
        };
        app.refresh_styles();
        Ok(app)
    }

    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        while !self.quit {
            // Expire any status message whose timeout has elapsed.
            if let Some(deadline) = self.status_msg_until {
                if Instant::now() >= deadline {
                    self.clear_status();
                }
            }
            terminal.draw(|f| self.draw(f))?;

            // Block on input, but wake up to clear the status message.
            let timeout = self
                .status_msg_until
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_secs(3600));
            if event::poll(timeout)? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.handle_key(k);
                    }
                }
            }
            self.refresh_styles();
        }
        self.persist()?;
        Ok(())
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some(msg.into());
        self.status_msg_until = Some(Instant::now() + STATUS_MSG_DURATION);
    }

    fn clear_status(&mut self) {
        self.status_msg = None;
        self.status_msg_until = None;
    }

    fn handle_key(&mut self, k: KeyEvent) {
        use crossterm::event::KeyModifiers as M;
        // Ex command mode owns all keystrokes until completed or cancelled.
        if self.ex_command.is_some() {
            self.handle_ex_key(k);
            return;
        }
        // A pending verb consumes the next keystroke as its target (or cancels).
        if let Some(verb) = self.pending_verb.take() {
            self.handle_verb_target(verb, k);
            return;
        }
        // Global: Ctrl-Q quits. Always wins, even in insert mode.
        if k.code == KeyCode::Char('q') && k.modifiers.contains(M::CONTROL) {
            self.quit = true;
            return;
        }
        // Global: Tab switches panes. Always wins, in any mode (no Tab inside bullets).
        if k.code == KeyCode::Tab && k.modifiers.is_empty() {
            self.focus = match self.focus {
                Pane::Did => Pane::Planning,
                Pane::Planning => Pane::Did,
            };
            return;
        }
        // Send to the focused vim buffer. If it returns false, the key is a
        // normal-mode app-level verb (q quit, y yank, : ex, <space> leader).
        let buf = self.focused_buf();
        let consumed = buf.input(k);
        if !consumed {
            self.handle_app_verb(k);
        }
    }

    fn handle_app_verb(&mut self, k: KeyEvent) {
        if k.code == KeyCode::Char('q') && k.modifiers.is_empty() {
            self.quit = true;
            return;
        }
        if k.code == KeyCode::Char(':')
            && (k.modifiers.is_empty() || k.modifiers == crossterm::event::KeyModifiers::SHIFT)
        {
            self.ex_command = Some(String::new());
            return;
        }
        if k.code == KeyCode::Char('y') && k.modifiers.is_empty() {
            self.pending_verb = Some(Verb::Yank);
            return;
        }
        // <space> leader is wired up when we add the help overlay.
    }

    fn handle_verb_target(&mut self, verb: Verb, k: KeyEvent) {
        // Esc or any non-target key cancels silently.
        match (verb, k.code) {
            (Verb::Yank, KeyCode::Char('t')) => self.yank_teams(),
            (Verb::Yank, KeyCode::Char('x')) => self.yank_xero(),
            (Verb::Yank, KeyCode::Char('d')) => self.yank_did(),
            (Verb::Yank, KeyCode::Char('p')) => self.yank_planning(),
            _ => {}
        }
    }

    fn yank_teams(&mut self) {
        let store = self.current_store_view();
        let text = format::standup(&store, self.today);
        self.copy_to_clipboard(text, "yanked teams");
    }

    fn yank_xero(&mut self) {
        let store = self.current_store_view();
        let text = format::timesheet(&store, self.yesterday);
        self.copy_to_clipboard(text, format!("yanked xero ({})", self.yesterday));
    }

    fn yank_did(&mut self) {
        let store = self.current_store_view();
        let did = store.get(self.yesterday).map(|e| &e.did[..]).unwrap_or(&[]);
        let text = format::bullets(did);
        self.copy_to_clipboard(text, "yanked yesterday's did");
    }

    fn yank_planning(&mut self) {
        let store = self.current_store_view();
        let planning = store
            .get(self.today)
            .map(|e| &e.planning[..])
            .unwrap_or(&[]);
        let text = format::bullets(planning);
        self.copy_to_clipboard(text, "yanked today's planning");
    }

    fn copy_to_clipboard(&mut self, text: String, ok_msg: impl Into<String>) {
        match clipboard::copy(&text) {
            Ok(()) => self.set_status(ok_msg),
            Err(e) => self.set_status(format!("E: clipboard: {e}")),
        }
    }

    /// A snapshot of the store with the current buffer contents applied.
    /// Used by the yank verb so we copy what's on-screen, not what's on disk.
    fn current_store_view(&self) -> Store {
        let mut s = self.store.clone();
        self.apply_buffer_state(&mut s);
        s
    }

    fn apply_buffer_state(&self, store: &mut Store) {
        let did = collect_bullets(&self.did_buf.area);
        let planning = collect_bullets(&self.planning_buf.area);
        if did.is_empty() {
            if let Some(e) = store.entries.get_mut(&self.yesterday) {
                e.did.clear();
            }
        } else {
            store.entry_mut(self.yesterday).did = did;
        }
        if planning.is_empty() {
            if let Some(e) = store.entries.get_mut(&self.today) {
                e.planning.clear();
            }
        } else {
            store.entry_mut(self.today).planning = planning;
        }
    }

    fn handle_ex_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Esc => {
                self.ex_command = None;
            }
            KeyCode::Enter => {
                if let Some(cmd) = self.ex_command.take() {
                    self.execute_ex(&cmd);
                }
            }
            KeyCode::Backspace => {
                if let Some(cmd) = self.ex_command.as_mut() {
                    if cmd.pop().is_none() {
                        // Backspace on empty ex line cancels (matches vim).
                        self.ex_command = None;
                    }
                }
            }
            KeyCode::Char(c) => {
                if let Some(cmd) = self.ex_command.as_mut() {
                    cmd.push(c);
                }
            }
            _ => {}
        }
    }

    fn execute_ex(&mut self, cmd: &str) {
        match cmd.trim() {
            "w" => match self.persist() {
                Ok(()) => self.set_status("written"),
                Err(e) => self.set_status(format!("E: save failed: {e}")),
            },
            "wq" | "x" => match self.persist() {
                Ok(()) => self.quit = true,
                Err(e) => self.set_status(format!("E: save failed: {e}")),
            },
            "q" => self.quit = true,
            "" => {}
            other => self.set_status(format!("E: unknown command \"{other}\"")),
        }
    }

    fn focused_buf(&mut self) -> &mut VimBuffer<'a> {
        match self.focus {
            Pane::Did => &mut self.did_buf,
            Pane::Planning => &mut self.planning_buf,
        }
    }

    fn focused_mode(&self) -> Mode {
        match self.focus {
            Pane::Did => self.did_buf.mode,
            Pane::Planning => self.planning_buf.mode,
        }
    }

    fn refresh_styles(&mut self) {
        let did_title = format!(" Yesterday ({}) — what I did ", self.yesterday);
        let planning_title = format!(" Today ({}) — planning ", self.today);
        apply_focus(
            &mut self.did_buf.area,
            did_title,
            self.focus == Pane::Did,
            self.did_buf.mode,
        );
        apply_focus(
            &mut self.planning_buf.area,
            planning_title,
            self.focus == Pane::Planning,
            self.planning_buf.mode,
        );
    }

    fn draw(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
            .split(f.area());
        self.draw_header(f, chunks[0]);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[1]);

        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);

        self.draw_yesterday_planning(f, top[0]);
        f.render_widget(&self.did_buf.area, top[1]);
        f.render_widget(&self.planning_buf.area, rows[1]);

        self.draw_status(f, chunks[2]);
    }

    fn draw_yesterday_planning(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(format!(" Yesterday ({}) — planning (ref) ", self.yesterday));
        let body = if self.yesterday_planning.is_empty() {
            vec![Line::from("(no planning recorded)").style(Style::default().fg(Color::DarkGray))]
        } else {
            self.yesterday_planning
                .iter()
                .map(|b| Line::from(format!("- {b}")))
                .collect()
        };
        let p = Paragraph::new(body).block(block).wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let header = Paragraph::new(Line::from(format!(" standup — {} ", self.today)))
            .style(Style::default().add_modifier(Modifier::BOLD));
        f.render_widget(header, area);
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        // Ex command line takes priority over mode/hint display.
        if let Some(cmd) = &self.ex_command {
            let line = Line::from(format!(":{cmd}"));
            f.render_widget(Paragraph::new(line), area);
            f.set_cursor_position((area.x + 1 + cmd.len() as u16, area.y));
            return;
        }
        if let Some(msg) = &self.status_msg {
            let line = Line::from(Span::styled(
                msg.as_str(),
                Style::default().fg(Color::Yellow),
            ));
            f.render_widget(Paragraph::new(line), area);
            return;
        }
        if let Some(Verb::Yank) = self.pending_verb {
            let line = Line::from(vec![
                Span::styled(
                    " YANK ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " t: teams    x: xero    d: did    p: planning    (any other key cancels) ",
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            f.render_widget(Paragraph::new(line), area);
            return;
        }
        let mode = self.focused_mode();
        let (mode_fg, mode_bg) = match mode {
            Mode::Normal => (Color::Black, Color::Green),
            Mode::Insert => (Color::Black, Color::Yellow),
        };
        let hint = match mode {
            Mode::Normal => " i/a/o insert  hjkl/w/b/e/0/$ move  gg/G ends  dd/cc/D/C/x del  u/Ctrl-R undo  y{t,x,d,p} yank  :w/:wq/:q  Tab switch  q quit ",
            Mode::Insert => " Esc/jj: normal    Ctrl-Backspace: delete word    Tab: switch ",
        };
        let line = Line::from(vec![
            Span::styled(
                format!(" {} ", mode.label()),
                Style::default()
                    .fg(mode_fg)
                    .bg(mode_bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn persist(&mut self) -> Result<()> {
        let mut store = self.store.clone();
        self.apply_buffer_state(&mut store);
        storage::save(&self.path, &store)?;
        self.store = store;
        Ok(())
    }
}

fn make_textarea<'a>(lines: Vec<String>) -> TextArea<'a> {
    let mut buf = if lines.is_empty() {
        TextArea::default()
    } else {
        TextArea::new(lines)
    };
    buf.move_cursor(CursorMove::Bottom);
    buf.move_cursor(CursorMove::End);
    buf
}

fn apply_focus(buf: &mut TextArea<'_>, title: String, focused: bool, mode: Mode) {
    let border = if focused { Color::Yellow } else { Color::DarkGray };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .title(title);
    buf.set_block(block);
    buf.set_cursor_line_style(Style::default());
    // Line numbers act as our trailing-blank indicator: empty lines past the
    // last bullet still show a number.
    buf.set_line_number_style(Style::default().fg(Color::DarkGray));
    if focused {
        // Block cursor in normal mode, thin underline in insert mode.
        let style = match mode {
            Mode::Normal => Style::default().add_modifier(Modifier::REVERSED),
            Mode::Insert => Style::default().add_modifier(Modifier::UNDERLINED),
        };
        buf.set_cursor_style(style);
    } else {
        buf.set_cursor_style(Style::default());
    }
}

fn collect_bullets(buf: &TextArea<'_>) -> Vec<String> {
    buf.lines()
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
