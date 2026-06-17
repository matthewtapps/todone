use std::cell::Cell;

use chrono::NaiveDate;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::calendar::CalendarEvent;
use crate::gitlab::{RawEvent, summarise};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Planning,
    Gitlab,
    Calendar,
}

const TAB_ORDER: [Tab; 3] = [Tab::Planning, Tab::Gitlab, Tab::Calendar];

fn tab_index(t: Tab) -> usize {
    match t {
        Tab::Planning => 0,
        Tab::Gitlab => 1,
        Tab::Calendar => 2,
    }
}

pub struct ContextState {
    pub tab: Tab,
    /// Per-tab scroll offset (first visible row).
    scrolls: Cell<[u16; 3]>,
    /// Per-tab max scroll offset, cached from the last render. Used to clamp
    /// scroll commands; interior-mutable so draw can update it via `&self`.
    max_scrolls: Cell<[u16; 3]>,
}

impl ContextState {
    pub fn new(initial: Tab) -> Self {
        Self {
            tab: initial,
            scrolls: Cell::new([0; 3]),
            max_scrolls: Cell::new([0; 3]),
        }
    }

    pub fn cycle(&mut self, delta: i32) {
        let cur = TAB_ORDER
            .iter()
            .position(|t| *t == self.tab)
            .unwrap_or(0) as i32;
        let n = TAB_ORDER.len() as i32;
        let next = (cur + delta).rem_euclid(n) as usize;
        self.tab = TAB_ORDER[next];
    }

    fn current_scroll(&self) -> u16 {
        self.scrolls.get()[tab_index(self.tab)]
    }

    /// Scroll the active tab by `delta` rows, clamped to `[0, max_scroll]`.
    pub fn scroll_by(&self, delta: i32) {
        let idx = tab_index(self.tab);
        let max = self.max_scrolls.get()[idx] as i32;
        let mut scrolls = self.scrolls.get();
        let new = (scrolls[idx] as i32 + delta).clamp(0, max) as u16;
        scrolls[idx] = new;
        self.scrolls.set(scrolls);
    }

    pub fn scroll_to_top(&self) {
        let idx = tab_index(self.tab);
        let mut scrolls = self.scrolls.get();
        scrolls[idx] = 0;
        self.scrolls.set(scrolls);
    }

    pub fn scroll_to_bottom(&self) {
        let idx = tab_index(self.tab);
        let max = self.max_scrolls.get()[idx];
        let mut scrolls = self.scrolls.get();
        scrolls[idx] = max;
        self.scrolls.set(scrolls);
    }

    /// Record the maximum scroll for `tab` after a render, and clamp the
    /// stored offset down to it.
    fn update_max_scroll(&self, tab: Tab, max: u16) {
        let idx = tab_index(tab);
        let mut maxes = self.max_scrolls.get();
        maxes[idx] = max;
        self.max_scrolls.set(maxes);
        let mut scrolls = self.scrolls.get();
        if scrolls[idx] > max {
            scrolls[idx] = max;
            self.scrolls.set(scrolls);
        }
    }
}

pub enum GitlabPaneStatus<'a> {
    Disabled,
    Loading,
    Events(&'a [RawEvent]),
    Error(&'a str),
}

pub enum CalendarPaneStatus<'a> {
    Disabled,
    Loading,
    Error(&'a str),
    Events {
        yesterday: &'a [CalendarEvent],
        today: &'a [CalendarEvent],
    },
}

pub fn draw(
    f: &mut Frame,
    area: Rect,
    state: &ContextState,
    yesterday: NaiveDate,
    viewing_date: NaiveDate,
    planning: &[String],
    gitlab: GitlabPaneStatus,
    calendar: CalendarPaneStatus,
    focused: bool,
) {
    let gitlab_count = match &gitlab {
        GitlabPaneStatus::Events(e) => Some(e.len()),
        _ => None,
    };
    let calendar_counts = match &calendar {
        CalendarPaneStatus::Events { yesterday, today } => Some((yesterday.len(), today.len())),
        _ => None,
    };

    let border_color = if focused { Color::Yellow } else { Color::DarkGray };
    let title = format!(" Context — {} ", format_date(yesterday));
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    draw_tabs(f, chunks[0], state, gitlab_count, calendar_counts);

    if chunks.len() < 2 {
        return;
    }
    match state.tab {
        Tab::Planning => draw_planning(f, chunks[1], planning, state),
        Tab::Gitlab => draw_gitlab(f, chunks[1], &gitlab, state),
        Tab::Calendar => draw_calendar(f, chunks[1], &calendar, yesterday, viewing_date, state),
    }
}

fn draw_tabs(
    f: &mut Frame,
    area: Rect,
    state: &ContextState,
    gitlab_count: Option<usize>,
    calendar_counts: Option<(usize, usize)>,
) {
    let planning_label = "Yesterday Planning";
    let gitlab_label = match gitlab_count {
        Some(n) => format!("GitLab ({n})"),
        None => "GitLab".to_string(),
    };
    let calendar_label = match calendar_counts {
        Some((y, t)) => format!("Calendar ({y}/{t})"),
        None => "Calendar".to_string(),
    };

    let active = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let inactive = Style::default().fg(Color::DarkGray);

    let spans = vec![
        Span::raw(" "),
        Span::styled(
            planning_label.to_string(),
            if state.tab == Tab::Planning { active } else { inactive },
        ),
        Span::raw("    "),
        Span::styled(
            gitlab_label,
            if state.tab == Tab::Gitlab { active } else { inactive },
        ),
        Span::raw("    "),
        Span::styled(
            calendar_label,
            if state.tab == Tab::Calendar { active } else { inactive },
        ),
        Span::styled(
            "        [ ] switch tab",
            Style::default().fg(Color::DarkGray),
        ),
    ];
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_planning(f: &mut Frame, area: Rect, planning: &[String], state: &ContextState) {
    let width = area.width as usize;
    let body: Vec<Line> = if planning.is_empty() {
        vec![Line::from(Span::styled(
            " (no planning recorded)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        planning
            .iter()
            .flat_map(|b| {
                let (level, content) = crate::format::parse_item(b);
                let prefix = if level == 0 { " - " } else { "   - " };
                wrap_with_prefix(prefix, content, Style::default(), width)
            })
            .collect()
    };
    let max = (body.len() as u16).saturating_sub(area.height);
    state.update_max_scroll(Tab::Planning, max);
    let scroll = state.current_scroll();
    f.render_widget(Paragraph::new(body).scroll((scroll, 0)), area);
}

fn draw_gitlab(f: &mut Frame, area: Rect, status: &GitlabPaneStatus, state: &ContextState) {
    let width = area.width as usize;
    let body = match status {
        GitlabPaneStatus::Disabled => vec![Line::from(Span::styled(
            " GitLab integration disabled — enable in <Space>s settings",
            Style::default().fg(Color::DarkGray),
        ))],
        GitlabPaneStatus::Loading => vec![Line::from(Span::styled(
            " fetching…",
            Style::default().fg(Color::DarkGray),
        ))],
        GitlabPaneStatus::Error(msg) => vec![Line::from(Span::styled(
            format!(" error: {msg}"),
            Style::default().fg(Color::Red),
        ))],
        GitlabPaneStatus::Events(events) => {
            let groups = summarise(events);
            if groups.is_empty() {
                vec![Line::from(Span::styled(
                    " no activity recorded",
                    Style::default().fg(Color::DarkGray),
                ))]
            } else {
                let mut body = Vec::new();
                for (i, group) in groups.into_iter().enumerate() {
                    if i > 0 {
                        body.push(Line::from(""));
                    }
                    body.push(Line::from(Span::styled(
                        format!(" {}", group.project),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                    for line in group.lines {
                        body.extend(wrap_gitlab_line(line.icon, &line.text, width));
                    }
                }
                body
            }
        }
    };
    let max = (body.len() as u16).saturating_sub(area.height);
    state.update_max_scroll(Tab::Gitlab, max);
    let scroll = state.current_scroll();
    f.render_widget(Paragraph::new(body).scroll((scroll, 0)), area);
}

fn draw_calendar(
    f: &mut Frame,
    area: Rect,
    status: &CalendarPaneStatus,
    yesterday: NaiveDate,
    today: NaiveDate,
    state: &ContextState,
) {
    let width = area.width as usize;
    match status {
        CalendarPaneStatus::Disabled => {
            state.update_max_scroll(Tab::Calendar, 0);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    " Calendar integration disabled — enable in <Space>s settings",
                    Style::default().fg(Color::DarkGray),
                ))),
                area,
            );
        }
        CalendarPaneStatus::Loading => {
            state.update_max_scroll(Tab::Calendar, 0);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    " fetching…",
                    Style::default().fg(Color::DarkGray),
                ))),
                area,
            );
        }
        CalendarPaneStatus::Error(msg) => {
            state.update_max_scroll(Tab::Calendar, 0);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" error: {msg}"),
                    Style::default().fg(Color::Red),
                ))),
                area,
            );
        }
        CalendarPaneStatus::Events { yesterday: y_events, today: t_events } => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            let col_width = (width / 2).max(1);
            let scroll = state.current_scroll();
            let y_len = draw_calendar_column(f, cols[0], yesterday, y_events, col_width, scroll);
            let t_len = draw_calendar_column(f, cols[1], today, t_events, col_width, scroll);
            // Both columns scroll together; clamp to whichever column is longer
            // so the user can always reach the bottom of either.
            let max = y_len.max(t_len).saturating_sub(area.height);
            state.update_max_scroll(Tab::Calendar, max);
        }
    }
}

fn draw_calendar_column(
    f: &mut Frame,
    area: Rect,
    date: NaiveDate,
    events: &[CalendarEvent],
    width: usize,
    scroll: u16,
) -> u16 {
    let mut body = Vec::new();
    body.push(Line::from(Span::styled(
        format!(" {}", format_date(date)),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    if events.is_empty() {
        body.push(Line::from(Span::styled(
            "   (no events)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for ev in events {
            body.extend(calendar_event_lines(ev, width));
        }
    }
    let total = body.len() as u16;
    f.render_widget(Paragraph::new(body).scroll((scroll, 0)), area);
    total
}

/// Format one event as: " HH:MM-HH:MM  Summary" (or "  all day    Summary").
/// Continuation rows of a wrapped summary hang-indent under the summary column.
fn calendar_event_lines<'a>(ev: &CalendarEvent, width: usize) -> Vec<Line<'a>> {
    const LEFT: usize = 1;
    const GAP: usize = 2;
    let time_label = format_time_range(ev);
    let prefix_width = LEFT + time_label.chars().count() + GAP;
    let body_width = width.saturating_sub(prefix_width).max(1);
    let segments = wrap_words(&ev.summary, body_width);
    let indent = " ".repeat(prefix_width);
    let time_style = if ev.all_day {
        Style::default().fg(Color::Magenta)
    } else {
        Style::default().fg(Color::Green)
    };
    segments
        .into_iter()
        .enumerate()
        .map(|(i, seg)| {
            if i == 0 {
                Line::from(vec![
                    Span::raw(" ".repeat(LEFT)),
                    Span::styled(time_label.clone(), time_style),
                    Span::raw(" ".repeat(GAP)),
                    Span::raw(seg),
                ])
            } else {
                Line::from(vec![Span::raw(indent.clone()), Span::raw(seg)])
            }
        })
        .collect()
}

/// 11 chars wide for both modes so columns line up.
fn format_time_range(ev: &CalendarEvent) -> String {
    if ev.all_day {
        "all day    ".to_string()
    } else {
        format!(
            "{}-{}",
            ev.start.format("%H:%M"),
            ev.end.format("%H:%M")
        )
    }
}

/// Wrap a single GitLab summary line so continuation rows align under the
/// text column (past the icon), matching the unfocused-editor hang style.
fn wrap_gitlab_line<'a>(icon: &'static str, text: &str, width: usize) -> Vec<Line<'a>> {
    // Visual layout: 4 left margin + icon (1 cell) + 2 spaces + text.
    const LEFT: usize = 4;
    const ICON_GAP: usize = 2;
    let icon_width = icon.chars().count().max(1);
    let prefix_width = LEFT + icon_width + ICON_GAP;
    let body_width = width.saturating_sub(prefix_width).max(1);
    let segments = wrap_words(text, body_width);
    let indent = " ".repeat(prefix_width);
    segments
        .into_iter()
        .enumerate()
        .map(|(i, seg)| {
            if i == 0 {
                Line::from(vec![
                    Span::raw(" ".repeat(LEFT)),
                    Span::styled(icon, Style::default().fg(Color::Cyan)),
                    Span::raw(" ".repeat(ICON_GAP)),
                    Span::raw(seg),
                ])
            } else {
                Line::from(vec![Span::raw(indent.clone()), Span::raw(seg)])
            }
        })
        .collect()
}

/// Wrap `text` so the first line carries `prefix` and continuation lines are
/// padded with spaces of the same visible width. Used for the planning tab.
fn wrap_with_prefix<'a>(
    prefix: &'static str,
    text: &str,
    style: Style,
    width: usize,
) -> Vec<Line<'a>> {
    let prefix_width = prefix.chars().count();
    let body_width = width.saturating_sub(prefix_width).max(1);
    let segments = wrap_words(text, body_width);
    let indent = " ".repeat(prefix_width);
    segments
        .into_iter()
        .enumerate()
        .map(|(i, seg)| {
            let head = if i == 0 { prefix.to_string() } else { indent.clone() };
            Line::from(vec![Span::styled(head, style), Span::styled(seg, style)])
        })
        .collect()
}

/// Greedy word-wrap. Whitespace is collapsed; a word longer than `width` is
/// emitted on its own line and allowed to overflow rather than being broken
/// mid-word.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        let current_len = current.chars().count();
        if current.is_empty() {
            current.push_str(word);
        } else if current_len + 1 + word_len <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn format_date(d: NaiveDate) -> String {
    d.format("%a %b %-d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_wraps_in_both_directions() {
        let mut s = ContextState::new(Tab::Planning);
        s.cycle(1);
        assert_eq!(s.tab, Tab::Gitlab);
        s.cycle(1);
        assert_eq!(s.tab, Tab::Calendar);
        s.cycle(1);
        assert_eq!(s.tab, Tab::Planning);
        s.cycle(-1);
        assert_eq!(s.tab, Tab::Calendar);
    }
}
