use crossterm::event::{KeyCode, KeyEvent, KeyModifiers as M};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::config::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Gitlab,
}

const TOP_ITEMS: &[(Section, &str)] = &[(Section::Gitlab, "GitLab integration")];

#[derive(Debug, Clone, Copy)]
enum FieldKind {
    Toggle,
    Text,
    Secret,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Focus {
    Top { cursor: usize },
    Section { section: Section, cursor: usize, editing: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    /// Close the settings screen. `save` is true for Esc-out and :wq, false for :q.
    Close { save: bool },
    /// Persist now and remain in the settings screen.
    Save,
}

pub struct SettingsState {
    pub settings: Settings,
    focus: Focus,
    dirty: bool,
    pub ex_command: Option<String>,
}

impl SettingsState {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            focus: Focus::Top { cursor: 0 },
            dirty: false,
            ex_command: None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    pub fn editing(&self) -> bool {
        matches!(self.focus, Focus::Section { editing: true, .. })
    }

    pub fn mode_label(&self) -> &'static str {
        if self.editing() { "EDIT" } else { "NORMAL" }
    }

    pub fn handle_key(&mut self, k: KeyEvent) -> SettingsAction {
        if self.ex_command.is_some() {
            return self.handle_ex_key(k);
        }
        // Ex-command entry: only allowed when not editing a text field.
        if !self.editing()
            && k.code == KeyCode::Char(':')
            && (k.modifiers.is_empty() || k.modifiers == M::SHIFT)
        {
            self.ex_command = Some(String::new());
            return SettingsAction::None;
        }
        match self.focus {
            Focus::Top { cursor } => self.handle_top_key(k, cursor),
            Focus::Section { section, cursor, editing: false } => {
                self.handle_section_key(k, section, cursor)
            }
            Focus::Section { section, cursor, editing: true } => {
                self.handle_edit_key(k, section, cursor)
            }
        }
    }

    fn handle_top_key(&mut self, k: KeyEvent, cursor: usize) -> SettingsAction {
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => SettingsAction::Close { save: true },
            KeyCode::Char('j') | KeyCode::Down => {
                let next = (cursor + 1).min(TOP_ITEMS.len().saturating_sub(1));
                self.focus = Focus::Top { cursor: next };
                SettingsAction::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.focus = Focus::Top { cursor: cursor.saturating_sub(1) };
                SettingsAction::None
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(&(section, _)) = TOP_ITEMS.get(cursor) {
                    self.focus = Focus::Section { section, cursor: 0, editing: false };
                }
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn handle_section_key(
        &mut self,
        k: KeyEvent,
        section: Section,
        cursor: usize,
    ) -> SettingsAction {
        let n_fields = section_fields(section).len();
        match k.code {
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('q') | KeyCode::Left => {
                let top_cursor = TOP_ITEMS
                    .iter()
                    .position(|&(s, _)| s == section)
                    .unwrap_or(0);
                self.focus = Focus::Top { cursor: top_cursor };
                SettingsAction::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let next = (cursor + 1).min(n_fields.saturating_sub(1));
                self.focus = Focus::Section { section, cursor: next, editing: false };
                SettingsAction::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.focus = Focus::Section {
                    section,
                    cursor: cursor.saturating_sub(1),
                    editing: false,
                };
                SettingsAction::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate(section, cursor);
                SettingsAction::None
            }
            KeyCode::Char('i') | KeyCode::Char('a') => {
                if matches!(
                    section_field_kind(section, cursor),
                    Some(FieldKind::Text) | Some(FieldKind::Secret)
                ) {
                    self.focus = Focus::Section { section, cursor, editing: true };
                }
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn handle_edit_key(
        &mut self,
        k: KeyEvent,
        section: Section,
        cursor: usize,
    ) -> SettingsAction {
        match k.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.focus = Focus::Section { section, cursor, editing: false };
                SettingsAction::None
            }
            KeyCode::Backspace if k.modifiers.contains(M::CONTROL) => {
                delete_word(&mut self.settings, section, cursor);
                self.dirty = true;
                SettingsAction::None
            }
            KeyCode::Backspace => {
                pop_char(&mut self.settings, section, cursor);
                self.dirty = true;
                SettingsAction::None
            }
            KeyCode::Char(c) => {
                push_char(&mut self.settings, section, cursor, c);
                self.dirty = true;
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn handle_ex_key(&mut self, k: KeyEvent) -> SettingsAction {
        match k.code {
            KeyCode::Esc => {
                self.ex_command = None;
                SettingsAction::None
            }
            KeyCode::Enter => {
                let cmd = self.ex_command.take().unwrap_or_default();
                match cmd.trim() {
                    "w" => SettingsAction::Save,
                    "wq" | "x" => SettingsAction::Close { save: true },
                    "q" => SettingsAction::Close { save: false },
                    _ => SettingsAction::None,
                }
            }
            KeyCode::Backspace => {
                if let Some(cmd) = self.ex_command.as_mut() {
                    if cmd.pop().is_none() {
                        self.ex_command = None;
                    }
                }
                SettingsAction::None
            }
            KeyCode::Char(c) => {
                if let Some(cmd) = self.ex_command.as_mut() {
                    cmd.push(c);
                }
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn activate(&mut self, section: Section, cursor: usize) {
        match section_field_kind(section, cursor) {
            Some(FieldKind::Toggle) => {
                toggle_field(&mut self.settings, section, cursor);
                self.dirty = true;
            }
            Some(FieldKind::Text) | Some(FieldKind::Secret) => {
                self.focus = Focus::Section { section, cursor, editing: true };
            }
            None => {}
        }
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        match self.focus {
            Focus::Top { cursor } => self.draw_top(f, area, cursor),
            Focus::Section { section, cursor, editing } => {
                self.draw_section(f, area, section, cursor, editing)
            }
        }
    }

    fn draw_top(&self, f: &mut Frame, area: Rect, cursor: usize) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Settings ")
            .border_style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let items: Vec<Line> = TOP_ITEMS
            .iter()
            .enumerate()
            .map(|(i, (_, label))| {
                let focused = i == cursor;
                let marker = if focused { "›" } else { " " };
                let style = if focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::styled(format!(" {marker} "), style),
                    Span::styled(format!("{label}  ›"), style),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(items), inner);
    }

    fn draw_section(
        &self,
        f: &mut Frame,
        area: Rect,
        section: Section,
        cursor: usize,
        editing: bool,
    ) {
        let title = format!(" Settings › {} ", section_title(section));
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let fields = section_fields(section);
        let label_width = fields.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0);

        let items: Vec<Line> = fields
            .iter()
            .enumerate()
            .map(|(i, (label, kind))| {
                let focused = i == cursor;
                let marker = if focused { "›" } else { " " };
                let label_style = if focused {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let raw = field_value(&self.settings, section, i);
                let display = match kind {
                    FieldKind::Secret if !(editing && focused) => "•".repeat(raw.chars().count()),
                    _ => raw,
                };
                let value_style = if editing && focused {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::styled(format!(" {marker} "), label_style),
                    Span::styled(format!("{label:<label_width$}  "), label_style),
                    Span::styled(display, value_style),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(items), inner);
    }
}

fn section_title(section: Section) -> &'static str {
    match section {
        Section::Gitlab => "GitLab",
    }
}

fn section_fields(section: Section) -> &'static [(&'static str, FieldKind)] {
    match section {
        Section::Gitlab => &[
            ("Enabled", FieldKind::Toggle),
            ("Instance URL", FieldKind::Text),
            ("Personal Token", FieldKind::Secret),
            ("Username", FieldKind::Text),
        ],
    }
}

fn section_field_kind(section: Section, idx: usize) -> Option<FieldKind> {
    section_fields(section).get(idx).map(|(_, k)| *k)
}

fn field_value(settings: &Settings, section: Section, idx: usize) -> String {
    match (section, idx) {
        (Section::Gitlab, 0) => {
            if settings.gitlab.enabled { "[x]".into() } else { "[ ]".into() }
        }
        (Section::Gitlab, 1) => settings.gitlab.instance_url.clone(),
        (Section::Gitlab, 2) => settings.gitlab.token.clone(),
        (Section::Gitlab, 3) => settings.gitlab.username.clone(),
        _ => String::new(),
    }
}

fn toggle_field(settings: &mut Settings, section: Section, idx: usize) {
    if let (Section::Gitlab, 0) = (section, idx) {
        settings.gitlab.enabled = !settings.gitlab.enabled;
    }
}

fn text_field_mut(settings: &mut Settings, section: Section, idx: usize) -> Option<&mut String> {
    match (section, idx) {
        (Section::Gitlab, 1) => Some(&mut settings.gitlab.instance_url),
        (Section::Gitlab, 2) => Some(&mut settings.gitlab.token),
        (Section::Gitlab, 3) => Some(&mut settings.gitlab.username),
        _ => None,
    }
}

fn push_char(settings: &mut Settings, section: Section, idx: usize, c: char) {
    if let Some(s) = text_field_mut(settings, section, idx) {
        s.push(c);
    }
}

fn pop_char(settings: &mut Settings, section: Section, idx: usize) {
    if let Some(s) = text_field_mut(settings, section, idx) {
        s.pop();
    }
}

fn delete_word(settings: &mut Settings, section: Section, idx: usize) {
    let Some(s) = text_field_mut(settings, section, idx) else {
        return;
    };
    while s.chars().next_back().is_some_and(|c| c.is_whitespace()) {
        s.pop();
    }
    while s.chars().next_back().is_some_and(|c| !c.is_whitespace()) {
        s.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn special(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn enter_drills_into_submenu_and_h_returns() {
        let mut s = SettingsState::new(Settings::default());
        assert!(matches!(s.focus, Focus::Top { .. }));
        s.handle_key(special(KeyCode::Enter));
        assert!(matches!(s.focus, Focus::Section { .. }));
        s.handle_key(key('h'));
        assert!(matches!(s.focus, Focus::Top { .. }));
    }

    #[test]
    fn space_toggles_enabled_field() {
        let mut s = SettingsState::new(Settings::default());
        s.handle_key(special(KeyCode::Enter)); // drill in
        s.handle_key(key(' ')); // toggle enabled
        assert!(s.settings.gitlab.enabled);
        assert!(s.dirty());
        s.handle_key(key(' '));
        assert!(!s.settings.gitlab.enabled);
    }

    #[test]
    fn typing_into_text_field_updates_settings() {
        let mut s = SettingsState::new(Settings::default());
        s.handle_key(special(KeyCode::Enter)); // drill in
        s.handle_key(key('j')); // cursor -> Instance URL
        s.handle_key(special(KeyCode::Enter)); // edit
        s.handle_key(key('g'));
        s.handle_key(key('l'));
        s.handle_key(special(KeyCode::Esc)); // leave edit
        assert_eq!(s.settings.gitlab.instance_url, "gl");
        assert!(s.dirty());
    }

    #[test]
    fn esc_at_top_returns_close_with_save() {
        let mut s = SettingsState::new(Settings::default());
        let action = s.handle_key(special(KeyCode::Esc));
        assert_eq!(action, SettingsAction::Close { save: true });
    }

    #[test]
    fn ex_q_closes_without_save() {
        let mut s = SettingsState::new(Settings::default());
        s.handle_key(key(':'));
        s.handle_key(key('q'));
        let action = s.handle_key(special(KeyCode::Enter));
        assert_eq!(action, SettingsAction::Close { save: false });
    }

    #[test]
    fn ex_w_returns_save_action() {
        let mut s = SettingsState::new(Settings::default());
        s.handle_key(key(':'));
        s.handle_key(key('w'));
        let action = s.handle_key(special(KeyCode::Enter));
        assert_eq!(action, SettingsAction::Save);
    }
}
