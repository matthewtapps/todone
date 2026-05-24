use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::mpsc,
    thread,
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
use tokio::runtime::Handle;
use tui_textarea::{CursorMove, TextArea};

use crate::{
    clipboard, config, format,
    gitlab::{self, RawEvent},
    history::{HistoryAction, HistoryState},
    settings::{SettingsAction, SettingsState},
    storage::{self, Store, previous_workday},
    vim::{Mode, VimBuffer},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Did,
    Planning,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Yank,
    Leader,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Today,
    History,
    Settings,
}

/// Events flowing into the main loop from input and background tasks.
pub enum AppEvent {
    Input(KeyEvent),
    /// User lookup completed.
    GitlabUserResolved(std::result::Result<u64, String>),
    /// One date's events fetched (or failed).
    GitlabFetched {
        date: NaiveDate,
        result: std::result::Result<Vec<RawEvent>, String>,
    },
}

struct GitlabCacheEntry {
    events: Vec<RawEvent>,
    fetched_at: Instant,
}

/// How long a fetched events list is considered fresh.
const GITLAB_TTL: Duration = Duration::from_secs(30 * 60);
/// Workdays before today to optimistically prefetch on startup.
const GITLAB_PREFETCH_DAYS: usize = 7;

pub struct App<'a> {
    path: PathBuf,
    store: Store,
    /// The actual current date — fixed for the session, used to flag the
    /// "is this today" indicator on the header.
    today: NaiveDate,
    /// The date currently being edited. Starts at `today`; `</`>` move it.
    viewing_date: NaiveDate,
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
    /// `Some` when a verb/leader has been pressed and we're waiting for its target key.
    pending: Option<Pending>,
    screen: Screen,
    history: HistoryState,
    help_open: bool,
    config_path: PathBuf,
    settings_state: SettingsState,
    runtime: Handle,
    event_tx: mpsc::Sender<AppEvent>,
    event_rx: mpsc::Receiver<AppEvent>,
    gitlab_client: Option<gitlab::Client>,
    gitlab_user_id: Option<u64>,
    gitlab_cache: BTreeMap<NaiveDate, GitlabCacheEntry>,
    gitlab_in_flight: std::collections::HashSet<NaiveDate>,
    gitlab_resolve_in_flight: bool,
}

const STATUS_MSG_DURATION: Duration = Duration::from_secs(2);

impl<'a> App<'a> {
    pub fn new(path: PathBuf, runtime: Handle) -> Result<Self> {
        let store = storage::load(&path)?;
        let today = Local::now().date_naive();
        let viewing_date = today;
        let yesterday = previous_workday(viewing_date);

        let (yesterday_planning, did_lines, planning_lines) =
            load_view(&store, yesterday, viewing_date);

        let did_buf = VimBuffer::new(make_textarea(did_lines));
        let planning_buf = VimBuffer::new(make_textarea(planning_lines));

        let config_path = config::default_path()?;
        let settings = config::load(&config_path)?;

        let (event_tx, event_rx) = mpsc::channel::<AppEvent>();

        let mut app = Self {
            path,
            store,
            today,
            viewing_date,
            yesterday,
            yesterday_planning,
            did_buf,
            planning_buf,
            focus: Pane::Did,
            quit: false,
            ex_command: None,
            status_msg: None,
            status_msg_until: None,
            pending: None,
            screen: Screen::Today,
            history: HistoryState::new(today),
            help_open: false,
            config_path,
            settings_state: SettingsState::new(settings),
            runtime,
            event_tx,
            event_rx,
            gitlab_client: None,
            gitlab_user_id: None,
            gitlab_cache: BTreeMap::new(),
            gitlab_in_flight: Default::default(),
            gitlab_resolve_in_flight: false,
        };
        app.refresh_styles();
        app.init_gitlab();
        Ok(app)
    }

    /// Move the viewing date by `delta` calendar days. Saves the current
    /// buffer state to the store first, then reloads from the new date.
    fn navigate_days(&mut self, delta: i64) {
        self.go_to_date(self.viewing_date + chrono::Duration::days(delta));
    }

    fn go_to_date(&mut self, new_date: NaiveDate) {
        self.save_buffers_to_store();
        self.viewing_date = new_date;
        self.yesterday = previous_workday(new_date);
        let (yp, did_lines, planning_lines) =
            load_view(&self.store, self.yesterday, self.viewing_date);
        self.yesterday_planning = yp;
        self.did_buf = VimBuffer::new(make_textarea(did_lines));
        self.planning_buf = VimBuffer::new(make_textarea(planning_lines));
        // Reset focus to the did pane so the user lands on the editable
        // "what I did" target for the new day.
        self.focus = Pane::Did;
        // Warm the GitLab context for the new viewing date's yesterday.
        self.fetch_for_date(self.yesterday, false);
    }

    fn save_buffers_to_store(&mut self) {
        let mut store = std::mem::take(&mut self.store);
        self.apply_buffer_state(&mut store);
        self.store = store;
    }

    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        spawn_input_thread(self.event_tx.clone());

        while !self.quit {
            // Expire any status message whose timeout has elapsed.
            if let Some(deadline) = self.status_msg_until {
                if Instant::now() >= deadline {
                    self.clear_status();
                }
            }
            terminal.draw(|f| self.draw(f))?;

            // Block on the next app event, waking only to clear status.
            let timeout = self
                .status_msg_until
                .map(|d| d.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::from_secs(3600));
            match self.event_rx.recv_timeout(timeout) {
                Ok(ev) => self.handle_event(ev),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.refresh_styles();
        }
        self.persist()?;
        Ok(())
    }

    fn handle_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Input(k) => self.handle_key(k),
            AppEvent::GitlabUserResolved(result) => {
                self.gitlab_resolve_in_flight = false;
                match result {
                    Ok(uid) => {
                        self.gitlab_user_id = Some(uid);
                        self.kick_off_prefetch();
                    }
                    Err(e) => self.set_status(format!("E: gitlab user: {e}")),
                }
            }
            AppEvent::GitlabFetched { date, result } => {
                self.gitlab_in_flight.remove(&date);
                match result {
                    Ok(events) => {
                        let n = events.len();
                        if let Err(e) = dump_events(date, &events) {
                            self.set_status(format!(
                                "gitlab: {n} events for {date} (cache write E: {e})"
                            ));
                        } else {
                            self.set_status(format!("gitlab: {n} events for {date}"));
                        }
                        self.gitlab_cache.insert(
                            date,
                            GitlabCacheEntry { events, fetched_at: Instant::now() },
                        );
                    }
                    Err(e) => self.set_status(format!("E: gitlab fetch {date}: {e}")),
                }
            }
        }
    }

    /// Build a GitLab client from current settings (or clear it) and kick off
    /// user resolution. Called at startup and after settings save.
    fn init_gitlab(&mut self) {
        let cfg = &self.settings_state.settings.gitlab;
        if !cfg.enabled
            || cfg.instance_url.trim().is_empty()
            || cfg.token.trim().is_empty()
            || cfg.username.trim().is_empty()
        {
            self.gitlab_client = None;
            self.gitlab_user_id = None;
            return;
        }
        let client = match gitlab::Client::new(&cfg.instance_url, &cfg.token) {
            Ok(c) => c,
            Err(e) => {
                self.set_status(format!("E: gitlab client: {e}"));
                return;
            }
        };
        self.gitlab_client = Some(client.clone());
        self.gitlab_user_id = None;
        self.gitlab_resolve_in_flight = true;
        let username = cfg.username.clone();
        let tx = self.event_tx.clone();
        self.runtime.spawn(async move {
            let result = client
                .resolve_user(&username)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AppEvent::GitlabUserResolved(result));
        });
    }

    /// Optimistically fetch events for yesterday + the previous workdays so
    /// the user has context warm by the time they navigate.
    fn kick_off_prefetch(&mut self) {
        let mut d = previous_workday(self.viewing_date);
        for _ in 0..GITLAB_PREFETCH_DAYS {
            self.fetch_for_date(d, false);
            d = previous_workday(d);
        }
    }

    /// Fetch events for `date`. Skips if a fetch is already in flight or the
    /// cached entry is still fresh (unless `force`).
    fn fetch_for_date(&mut self, date: NaiveDate, force: bool) {
        let Some(client) = self.gitlab_client.clone() else { return };
        let Some(uid) = self.gitlab_user_id else { return };
        if self.gitlab_in_flight.contains(&date) {
            return;
        }
        if !force {
            if let Some(entry) = self.gitlab_cache.get(&date) {
                if entry.fetched_at.elapsed() < GITLAB_TTL {
                    return;
                }
            }
        }
        self.gitlab_in_flight.insert(date);
        let tx = self.event_tx.clone();
        self.runtime.spawn(async move {
            let result = client
                .fetch_events(uid, date)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AppEvent::GitlabFetched { date, result });
        });
    }

    /// Refresh GitLab events for the date currently summarised (yesterday of
    /// the viewing date). Triggered by `<Space>r`.
    fn refresh_gitlab(&mut self) {
        if self.gitlab_client.is_none() {
            self.set_status("gitlab not configured");
            return;
        }
        if self.gitlab_user_id.is_none() {
            if !self.gitlab_resolve_in_flight {
                self.init_gitlab();
            }
            self.set_status("gitlab: resolving user...");
            return;
        }
        let target = previous_workday(self.viewing_date);
        self.fetch_for_date(target, true);
        self.set_status(format!("gitlab: refreshing {target}"));
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
        // Any new keystroke dismisses the previous transient status message;
        // the 2s timeout is only for the idle case.
        self.clear_status();
        // Help overlay swallows the next keystroke and dismisses.
        if self.help_open {
            self.help_open = false;
            return;
        }
        // Ex command mode owns all keystrokes until completed or cancelled.
        if self.ex_command.is_some() {
            self.handle_ex_key(k);
            return;
        }
        // Global: Ctrl-Q quits. Always wins, on any screen and in any mode.
        if k.code == KeyCode::Char('q') && k.modifiers.contains(M::CONTROL) {
            self.quit = true;
            return;
        }
        match self.screen {
            Screen::Today => self.handle_today_key(k),
            Screen::History => self.handle_history_key(k),
            Screen::Settings => self.handle_settings_key(k),
        }
    }

    fn handle_settings_key(&mut self, k: KeyEvent) {
        // `?` opens help without leaving the settings screen — but only while
        // not editing a field, where it would otherwise be typed.
        if k.code == KeyCode::Char('?')
            && k.modifiers.is_empty()
            && !self.settings_state.editing()
            && self.settings_state.ex_command.is_none()
        {
            self.help_open = true;
            return;
        }
        let action = self.settings_state.handle_key(k);
        match action {
            SettingsAction::None => {}
            SettingsAction::Save => self.save_settings(),
            SettingsAction::Close { save } => {
                if save && self.settings_state.dirty() {
                    self.save_settings();
                }
                self.screen = Screen::Today;
            }
        }
    }

    fn save_settings(&mut self) {
        match config::save(&self.config_path, &self.settings_state.settings) {
            Ok(()) => {
                self.settings_state.mark_clean();
                self.set_status("settings saved");
                // Re-evaluate gitlab integration in case its config changed.
                self.init_gitlab();
            }
            Err(e) => self.set_status(format!("E: settings save failed: {e}")),
        }
    }

    fn open_settings(&mut self) {
        // Reload from disk so the user sees the canonical persisted state.
        let settings = config::load(&self.config_path).unwrap_or_default();
        self.settings_state = SettingsState::new(settings);
        self.screen = Screen::Settings;
    }

    fn handle_history_key(&mut self, k: KeyEvent) {
        // `?` is universal: opens help without leaving the history screen.
        if k.code == KeyCode::Char('?') && k.modifiers.is_empty() {
            self.help_open = true;
            return;
        }
        let action = self.history.handle_key(k, &self.store);
        match action {
            HistoryAction::None => {}
            HistoryAction::Close => self.screen = Screen::Today,
            HistoryAction::Select(date) => {
                self.go_to_date(date);
                self.screen = Screen::Today;
            }
        }
    }

    fn handle_today_key(&mut self, k: KeyEvent) {
        use crossterm::event::KeyModifiers as M;
        // A pending verb/leader consumes the next keystroke as its target.
        if let Some(p) = self.pending.take() {
            self.handle_pending(p, k);
            return;
        }
        // Tab switches panes. Always wins; we don't put Tab characters in bullets.
        if k.code == KeyCode::Tab && k.modifiers.is_empty() {
            self.focus = match self.focus {
                Pane::Did => Pane::Planning,
                Pane::Planning => Pane::Did,
            };
            return;
        }
        // Send to the focused vim buffer. If it returns false, the key is a
        // normal-mode app-level verb (q quit, y yank, : ex, <space> leader, </> nav).
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
            self.pending = Some(Pending::Yank);
            return;
        }
        if k.code == KeyCode::Char(' ') && k.modifiers.is_empty() {
            self.pending = Some(Pending::Leader);
            return;
        }
        let plain_or_shift = k.modifiers.is_empty()
            || k.modifiers == crossterm::event::KeyModifiers::SHIFT;
        if plain_or_shift && k.code == KeyCode::Char('<') {
            self.navigate_days(-1);
            return;
        }
        if plain_or_shift && k.code == KeyCode::Char('>') {
            self.navigate_days(1);
            return;
        }
        if plain_or_shift && k.code == KeyCode::Char('?') {
            self.help_open = true;
            return;
        }
    }

    fn handle_pending(&mut self, p: Pending, k: KeyEvent) {
        // Esc or any non-target key cancels silently.
        match (p, k.code) {
            (Pending::Yank, KeyCode::Char('t')) => self.yank_teams(),
            (Pending::Yank, KeyCode::Char('x')) => self.yank_xero(),
            (Pending::Yank, KeyCode::Char('d')) => self.yank_did(),
            (Pending::Yank, KeyCode::Char('p')) => self.yank_planning(),
            (Pending::Leader, KeyCode::Char('h')) => self.open_history(),
            (Pending::Leader, KeyCode::Char('s')) => self.open_settings(),
            (Pending::Leader, KeyCode::Char('r')) => self.refresh_gitlab(),
            (Pending::Leader, KeyCode::Char('?')) => self.help_open = true,
            _ => {}
        }
    }

    fn open_history(&mut self) {
        self.save_buffers_to_store();
        self.history = HistoryState::new(self.today);
        self.screen = Screen::History;
    }

    fn yank_teams(&mut self) {
        let store = self.current_store_view();
        let text = format::standup_html(&store, self.viewing_date);
        match clipboard::copy_html(&text) {
            Ok(()) => self.set_status("yanked teams"),
            Err(e) => self.set_status(format!("E: clipboard: {e}")),
        }
    }

    fn yank_xero(&mut self) {
        let store = self.current_store_view();
        let text = format::timesheet(&store, self.yesterday);
        self.copy_to_clipboard(
            text,
            format!("yanked xero ({})", format_date_short(self.yesterday)),
        );
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
            .get(self.viewing_date)
            .map(|e| &e.planning[..])
            .unwrap_or(&[]);
        let text = format::bullets(planning);
        self.copy_to_clipboard(text, "yanked planning");
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
            if let Some(e) = store.entries.get_mut(&self.viewing_date) {
                e.planning.clear();
            }
        } else {
            store.entry_mut(self.viewing_date).planning = planning;
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
        let did_title = format!(
            " Yesterday — {} — what I did ",
            format_date_short(self.yesterday)
        );
        let planning_title = format!(
            " Today — {} — planning ",
            format_date_short(self.viewing_date)
        );
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

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        self.draw_header(f, chunks[0]);

        match self.screen {
            Screen::Today => self.draw_today(f, chunks[1]),
            Screen::History => self.history.draw(f, chunks[1], &self.store),
            Screen::Settings => self.settings_state.draw(f, chunks[1]),
        }

        self.draw_status(f, chunks[2]);

        if self.help_open {
            crate::help::draw(f, area);
        }
    }

    fn draw_today(&self, f: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(rows[0]);

        self.draw_yesterday_planning(f, top[0]);
        self.draw_buffer(f, top[1], &self.did_buf, self.focus == Pane::Did);
        self.draw_buffer(f, rows[1], &self.planning_buf, self.focus == Pane::Planning);
    }

    /// Focused pane renders the live `TextArea` (horizontal scroll, cursor).
    /// Unfocused pane renders the same content as a wrapped `Paragraph` with
    /// a hanging indent on continuation lines.
    fn draw_buffer(&self, f: &mut Frame, area: Rect, buf: &VimBuffer<'_>, focused: bool) {
        if focused {
            f.render_widget(&buf.area, area);
            return;
        }
        let block = buf.area.block().cloned();
        let inner_width = block
            .as_ref()
            .map(|b| b.inner(area).width)
            .unwrap_or(area.width) as usize;
        let body: Vec<Line> = buf
            .area
            .lines()
            .iter()
            .flat_map(|l| wrap_with_hang(l, inner_width, 2))
            .map(Line::from)
            .collect();
        let mut p = Paragraph::new(body);
        if let Some(b) = block {
            p = p.block(b);
        }
        f.render_widget(p, area);
    }

    fn draw_yesterday_planning(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(format!(
                " Yesterday — {} — planning (ref) ",
                format_date_short(self.yesterday)
            ));
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
        let suffix = if self.viewing_date == self.today {
            String::new()
        } else {
            let delta = (self.viewing_date - self.today).num_days();
            if delta > 0 {
                format!("  (+{delta}d)")
            } else {
                format!("  ({delta}d)")
            }
        };
        let date_color = if self.viewing_date == self.today {
            Color::Green
        } else {
            Color::Yellow
        };
        let line = Line::from(vec![
            Span::styled(" standup — ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format_date_short(self.viewing_date),
                Style::default()
                    .fg(date_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(suffix, Style::default().fg(Color::Yellow)),
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn draw_status(&self, f: &mut Frame, area: Rect) {
        // Settings screen has its own ex-command line; check first since the
        // app-level ex line is only set on the today screen.
        if self.screen == Screen::Settings {
            if let Some(cmd) = &self.settings_state.ex_command {
                let line = Line::from(format!(":{cmd}"));
                f.render_widget(Paragraph::new(line), area);
                f.set_cursor_position((area.x + 1 + cmd.len() as u16, area.y));
                return;
            }
        }
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
        match self.pending {
            Some(Pending::Yank) => {
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
            Some(Pending::Leader) => {
                let line = Line::from(vec![
                    Span::styled(
                        " LEADER ",
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " h: history    s: settings    r: refresh gitlab    (any other key cancels) ",
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                f.render_widget(Paragraph::new(line), area);
                return;
            }
            None => {}
        }
        if self.screen == Screen::Settings {
            let mode = self.settings_state.mode_label();
            let (mode_fg, mode_bg) = if mode == "EDIT" {
                (Color::Black, Color::Yellow)
            } else {
                (Color::Black, Color::Cyan)
            };
            let hint = if mode == "EDIT" {
                " Esc/Enter exit edit    Ctrl-Bksp del word "
            } else {
                " j/k move    h back    l/Enter enter/edit    Space toggle    Esc save+close    :w save    ?: keybinds "
            };
            let line = Line::from(vec![
                Span::styled(
                    format!(" {mode} "),
                    Style::default()
                        .fg(mode_fg)
                        .bg(mode_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
            ]);
            f.render_widget(Paragraph::new(line), area);
            return;
        }
        if self.screen == Screen::History {
            let mut spans = vec![
                Span::styled(
                    format!(" HISTORY ({}) ", self.history.view_label()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    " j/k move    Enter select    ?: keybinds    q back ",
                    Style::default().fg(Color::DarkGray),
                ),
            ];
            if let Some(n) = self.history.count_display() {
                spans.push(Span::styled(
                    format!("    [{n}] "),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), area);
            return;
        }
        let mode = self.focused_mode();
        let (mode_fg, mode_bg) = match mode {
            Mode::Normal => (Color::Black, Color::Green),
            Mode::Insert => (Color::Black, Color::Yellow),
        };
        let hint = match mode {
            Mode::Normal => " i insert    </> day    y yank    <space>h history    ?: keybinds    q quit ",
            Mode::Insert => " Esc/jj normal    Tab switch    ?: keybinds ",
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

fn load_view(
    store: &Store,
    yesterday: NaiveDate,
    viewing_date: NaiveDate,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let yesterday_planning = store
        .get(yesterday)
        .map(|e| e.planning.clone())
        .unwrap_or_default();
    // Yesterday's `did`: existing did, else its planning as a draft prompt.
    let did_lines = store
        .get(yesterday)
        .map(|e| if e.did.is_empty() { e.planning.clone() } else { e.did.clone() })
        .unwrap_or_default();
    let planning_lines = store
        .get(viewing_date)
        .map(|e| e.planning.clone())
        .unwrap_or_default();
    (yesterday_planning, did_lines, planning_lines)
}

/// Display date used everywhere in the UI, e.g. `Fri May 22`. Year dropped per design.
fn format_date_short(d: NaiveDate) -> String {
    d.format("%a %b %-d").to_string()
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

/// Spawn a daemon thread that forwards crossterm key presses onto `tx`. The
/// thread exits when the channel is dropped (i.e. on app shutdown).
fn spawn_input_thread(tx: mpsc::Sender<AppEvent>) {
    thread::spawn(move || loop {
        let Ok(ev) = event::read() else { return };
        if let Event::Key(k) = ev {
            if k.kind == KeyEventKind::Press {
                if tx.send(AppEvent::Input(k)).is_err() {
                    return;
                }
            }
        }
    });
}

/// Persist raw events for `date` to ~/.cache/standup/gitlab/{date}.json so
/// the user can inspect the API output during phase 2. Phase 3+ will surface
/// this in-app via the context pane.
fn dump_events(date: NaiveDate, events: &[RawEvent]) -> Result<()> {
    let dir = dirs::cache_dir()
        .ok_or_else(|| anyhow::anyhow!("no cache dir"))?
        .join("standup")
        .join("gitlab");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{date}.json"));
    let json = serde_json::to_string_pretty(events)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Word-wrap `text` to `width`, prepending `indent_spaces` spaces on every line
/// after the first. Whitespace inside `text` is collapsed to single spaces.
fn wrap_with_hang(text: &str, width: usize, indent_spaces: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let indent: String = " ".repeat(indent_spaces);
    let cont_width = width.saturating_sub(indent_spaces).max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let avail = if lines.is_empty() { width } else { cont_width };
        let word_len = word.chars().count();
        let current_len = current.chars().count();
        if current.is_empty() {
            current.push_str(word);
        } else if current_len + 1 + word_len <= avail {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(if lines.is_empty() {
                std::mem::take(&mut current)
            } else {
                format!("{indent}{}", std::mem::take(&mut current))
            });
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(if lines.is_empty() {
            current
        } else {
            format!("{indent}{current}")
        });
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn collect_bullets(buf: &TextArea<'_>) -> Vec<String> {
    buf.lines()
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}
