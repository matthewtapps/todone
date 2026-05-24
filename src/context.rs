use chrono::NaiveDate;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::gitlab::{RawEvent, summarise};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Planning,
    Gitlab,
}

pub struct ContextState {
    pub tab: Tab,
}

impl ContextState {
    pub fn new(initial: Tab) -> Self {
        Self { tab: initial }
    }

    pub fn cycle(&mut self, delta: i32) {
        let tabs = [Tab::Planning, Tab::Gitlab];
        let cur = tabs.iter().position(|t| *t == self.tab).unwrap_or(0) as i32;
        let n = tabs.len() as i32;
        let next = (cur + delta).rem_euclid(n) as usize;
        self.tab = tabs[next];
    }
}

pub enum GitlabPaneStatus<'a> {
    Disabled,
    Loading,
    Events(&'a [RawEvent]),
    Error(&'a str),
}

pub fn draw(
    f: &mut Frame,
    area: Rect,
    state: &ContextState,
    yesterday: NaiveDate,
    planning: &[String],
    gitlab: GitlabPaneStatus,
    focused: bool,
) {
    let event_count = match &gitlab {
        GitlabPaneStatus::Events(e) => Some(e.len()),
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

    draw_tabs(f, chunks[0], state, event_count);

    if chunks.len() < 2 {
        return;
    }
    match state.tab {
        Tab::Planning => draw_planning(f, chunks[1], planning),
        Tab::Gitlab => draw_gitlab(f, chunks[1], &gitlab),
    }
}

fn draw_tabs(f: &mut Frame, area: Rect, state: &ContextState, event_count: Option<usize>) {
    let planning_label = "Yesterday Planning";
    let gitlab_label = match event_count {
        Some(n) => format!("GitLab ({n})"),
        None => "GitLab".to_string(),
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
        Span::styled(
            "        [ ] switch tab",
            Style::default().fg(Color::DarkGray),
        ),
    ];
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_planning(f: &mut Frame, area: Rect, planning: &[String]) {
    let width = area.width as usize;
    let body: Vec<Line> = if planning.is_empty() {
        vec![Line::from(Span::styled(
            " (no planning recorded)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        planning
            .iter()
            .flat_map(|b| wrap_with_prefix(" - ", b, Style::default(), width))
            .collect()
    };
    f.render_widget(Paragraph::new(body), area);
}

fn draw_gitlab(f: &mut Frame, area: Rect, status: &GitlabPaneStatus) {
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
    f.render_widget(Paragraph::new(body), area);
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
        assert_eq!(s.tab, Tab::Planning);
        s.cycle(-1);
        assert_eq!(s.tab, Tab::Gitlab);
    }
}
