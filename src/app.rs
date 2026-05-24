use std::path::PathBuf;

use anyhow::Result;
use chrono::{Local, NaiveDate};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use tui_textarea::{CursorMove, TextArea};

use crate::storage::{self, Store, previous_workday};

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
    did_buf: TextArea<'a>,
    planning_buf: TextArea<'a>,
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

        let did_buf = make_textarea(did_lines);
        let planning_buf = make_textarea(planning_lines);

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
        }
        self.persist()?;
        Ok(())
    }

    fn handle_key(&mut self, k: KeyEvent) {
        use crossterm::event::KeyModifiers as M;
        // Quit: Ctrl-Q only for now (q is reserved for vim-normal-mode later).
        if k.code == KeyCode::Char('q') && k.modifiers.contains(M::CONTROL) {
            self.quit = true;
            return;
        }
        if k.code == KeyCode::Tab && k.modifiers.is_empty() {
            self.focus = match self.focus {
                Pane::Did => Pane::Planning,
                Pane::Planning => Pane::Did,
            };
            self.refresh_styles();
            return;
        }
        // Everything else goes to the focused buffer (raw editing for now;
        // vim modal layer wraps this in the next step).
        let buf = match self.focus {
            Pane::Did => &mut self.did_buf,
            Pane::Planning => &mut self.planning_buf,
        };
        // Ctrl+Backspace → delete previous word (must also be preserved in vim insert mode).
        if k.code == KeyCode::Backspace && k.modifiers.contains(M::CONTROL) {
            buf.delete_word();
            return;
        }
        buf.input(k);
    }

    fn refresh_styles(&mut self) {
        let did_title = format!(" Yesterday ({}) — what I did ", self.yesterday);
        let planning_title = format!(" Today ({}) — planning ", self.today);
        apply_focus(&mut self.did_buf, did_title, self.focus == Pane::Did);
        apply_focus(&mut self.planning_buf, planning_title, self.focus == Pane::Planning);
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
        f.render_widget(&self.did_buf, top[1]);
        f.render_widget(&self.planning_buf, rows[1]);

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
        let hint = " Tab: switch pane    Ctrl-Q: quit (saves) ";
        let status = Paragraph::new(Line::from(hint))
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(status, area);
    }

    fn persist(&mut self) -> Result<()> {
        let did = collect_bullets(&self.did_buf);
        let planning = collect_bullets(&self.planning_buf);

        // Yesterday's `did`. Update in place; only insert an entry if non-empty.
        if did.is_empty() {
            if let Some(e) = self.store.entries.get_mut(&self.yesterday) {
                e.did.clear();
            }
        } else {
            self.store.entry_mut(self.yesterday).did = did;
        }

        // Today's `planning`.
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

fn apply_focus(buf: &mut TextArea<'_>, title: String, focused: bool) {
    let border = if focused { Color::Yellow } else { Color::DarkGray };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .title(title);
    buf.set_block(block);
    buf.set_cursor_line_style(Style::default());
    if focused {
        buf.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
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
