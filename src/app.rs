use std::path::PathBuf;

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
    storage::{self, Store, previous_workday},
    vim::{Mode, VimBuffer},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Did,
    Planning,
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
}

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
        };
        app.refresh_styles();
        Ok(app)
    }

    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        while !self.quit {
            terminal.draw(|f| self.draw(f))?;
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    self.handle_key(k);
                }
            }
            self.refresh_styles();
        }
        self.persist()?;
        Ok(())
    }

    fn handle_key(&mut self, k: KeyEvent) {
        use crossterm::event::KeyModifiers as M;
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
        // normal-mode app-level verb (q quit, y yank, <space> leader).
        let buf = self.focused_buf();
        let consumed = buf.input(k);
        if !consumed {
            self.handle_app_verb(k);
        }
    }

    fn handle_app_verb(&mut self, k: KeyEvent) {
        if k.code == KeyCode::Char('q') && k.modifiers.is_empty() {
            self.quit = true;
        }
        // y and <space> wired up in the yank-verb step.
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
        let mode = self.focused_mode();
        let (mode_fg, mode_bg) = match mode {
            Mode::Normal => (Color::Black, Color::Green),
            Mode::Insert => (Color::Black, Color::Yellow),
        };
        let hint = match mode {
            Mode::Normal => " i/a/o: insert    hjkl/w/b/e/0/$: move    gg/G: top/bot    dd/cc/D/C/x: delete    u/Ctrl-R: undo    Tab: switch    q: quit ",
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
        let did = collect_bullets(&self.did_buf.area);
        let planning = collect_bullets(&self.planning_buf.area);

        if did.is_empty() {
            if let Some(e) = self.store.entries.get_mut(&self.yesterday) {
                e.did.clear();
            }
        } else {
            self.store.entry_mut(self.yesterday).did = did;
        }

        if planning.is_empty() {
            if let Some(e) = self.store.entries.get_mut(&self.today) {
                e.planning.clear();
            }
        } else {
            self.store.entry_mut(self.today).planning = planning;
        }

        storage::save(&self.path, &self.store)
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
