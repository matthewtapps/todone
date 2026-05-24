use chrono::{Duration, NaiveDate};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};

use crate::storage::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryAction {
    None,
    Close,
    /// Caller positions today screen so this date appears as yesterday's did.
    Select(NaiveDate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Bordered cards per day, multi-line cell contents. Default.
    Box,
    /// One single-line row per day. Useful for skimming long history.
    Compact,
}

pub struct HistoryState {
    today: NaiveDate,
    pub selected_date: NaiveDate,
    scroll_top: NaiveDate,
    initialized: bool,
    /// Pending vim sequence (e.g. `g` waiting for the second `g`).
    pending: Option<char>,
    /// Accumulated vim count prefix (e.g. `5j` → count = Some(5)).
    count: Option<u32>,
    view: ViewMode,
}

impl HistoryState {
    pub fn new(today: NaiveDate) -> Self {
        Self {
            today,
            selected_date: today,
            scroll_top: today,
            initialized: false,
            pending: None,
            count: None,
            view: ViewMode::Box,
        }
    }

    pub fn handle_key(&mut self, k: KeyEvent, store: &Store) -> HistoryAction {
        if let Some('g') = self.pending {
            self.pending = None;
            self.count = None;
            if k.code == KeyCode::Char('g') {
                if let Some(first) = store.first_recorded() {
                    self.selected_date = first;
                }
            }
            return HistoryAction::None;
        }

        // Digit prefix accumulates a count to apply to the next motion.
        if let KeyCode::Char(c) = k.code {
            if c.is_ascii_digit() && k.modifiers.is_empty() {
                // Leading `0` without an in-progress count is line-start in vim;
                // here we have no such motion, so we ignore it.
                let digit = c.to_digit(10).unwrap();
                if !(self.count.is_none() && digit == 0) {
                    self.count = Some(self.count.unwrap_or(0).saturating_mul(10) + digit);
                    return HistoryAction::None;
                }
            }
        }

        let count = self.count.take().unwrap_or(1) as i64;
        match k.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected_date += Duration::days(count);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected_date -= Duration::days(count);
            }
            KeyCode::Char('g') => {
                self.pending = Some('g');
            }
            KeyCode::Char('G') => {
                self.selected_date = self.today;
            }
            KeyCode::Char('z') => {
                self.view = match self.view {
                    ViewMode::Box => ViewMode::Compact,
                    ViewMode::Compact => ViewMode::Box,
                };
                // Re-center after view change so the cursor lands sensibly.
                self.initialized = false;
            }
            KeyCode::Enter => return HistoryAction::Select(self.selected_date),
            KeyCode::Esc | KeyCode::Char('q') => return HistoryAction::Close,
            _ => {}
        }
        HistoryAction::None
    }

    pub fn count_display(&self) -> Option<u32> {
        self.count
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        if area.height == 0 || area.width == 0 {
            return;
        }
        match self.view {
            ViewMode::Compact => self.draw_compact(f, area, store),
            ViewMode::Box => self.draw_box(f, area, store),
        }
    }

    pub fn view_label(&self) -> &'static str {
        match self.view {
            ViewMode::Box => "box",
            ViewMode::Compact => "compact",
        }
    }

    // ---------------- Compact view ----------------

    fn draw_compact(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        let row_count = area.height.saturating_sub(1) as usize; // 1 for header
        if row_count == 0 {
            return;
        }
        self.ensure_visible_rows(row_count as i64);

        let rows: Vec<Row> = (0..row_count)
            .map(|i| self.compact_row(i, store))
            .collect();
        let selected_idx = (self.selected_date - self.scroll_top).num_days() as usize;

        let header = Row::new(vec![
            Cell::from(""),
            Cell::from(" Date"),
            Cell::from(" Planned"),
            Cell::from(" Did"),
        ])
        .style(
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        );

        let table = Table::new(
            rows,
            [
                Constraint::Length(4),
                Constraint::Length(14),
                Constraint::Percentage(43),
                Constraint::Percentage(57),
            ],
        )
        .header(header)
        .row_highlight_style(
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .column_spacing(1);

        let mut state = TableState::default();
        state.select(Some(selected_idx));
        f.render_stateful_widget(table, area, &mut state);
    }

    fn compact_row(&self, i: usize, store: &Store) -> Row<'static> {
        let date = self.scroll_top + Duration::days(i as i64);
        let rel = relnum_label(date, self.selected_date);
        let (planned, did) = preview_join(store, date);
        let dim = Style::default().fg(Color::DarkGray);
        let date_label = format_date_short(date);
        let date_cell = if date == self.today {
            Cell::from(format!(" {date_label} *"))
                .style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        } else {
            Cell::from(format!(" {date_label}"))
        };
        Row::new(vec![
            Cell::from(Span::styled(rel, dim)),
            date_cell,
            empty_or(planned, "—", dim),
            empty_or(did, "—", dim),
        ])
    }

    // ---------------- Box view ----------------

    fn draw_box(&mut self, f: &mut Frame, area: Rect, store: &Store) {
        // Reserve a narrow column on the far left for relative line numbers
        // (rendered next to the top border of each card).
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(RELNUM_WIDTH), Constraint::Min(0)])
            .split(area);
        let relnum_area = split[0];
        let cards_area = split[1];

        let card_heights = self.box_card_layout(cards_area.height, store);
        if card_heights.is_empty() {
            return;
        }

        let mut constraints: Vec<Constraint> = card_heights
            .iter()
            .map(|(_, h)| Constraint::Length(*h))
            .collect();
        constraints.push(Constraint::Min(0));
        let row_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(cards_area);

        for (i, (date, _h)) in card_heights.iter().enumerate() {
            let card_area = row_areas[i];
            self.draw_relnum(f, relnum_area, card_area.y, *date);
            self.draw_card(f, card_area, *date, store);
        }
    }

    fn draw_relnum(&self, f: &mut Frame, relnum_area: Rect, row_y: u16, date: NaiveDate) {
        if row_y < relnum_area.y || row_y >= relnum_area.y + relnum_area.height {
            return;
        }
        let rect = Rect {
            x: relnum_area.x,
            y: row_y,
            width: relnum_area.width,
            height: 1,
        };
        let is_selected = date == self.selected_date;
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let text = relnum_label(date, self.selected_date);
        let para = Paragraph::new(Line::from(Span::styled(text, style)).alignment(Alignment::Right));
        f.render_widget(para, rect);
    }

    /// Plan which dates to render this frame and how tall each card is.
    fn box_card_layout(
        &mut self,
        area_height: u16,
        store: &Store,
    ) -> Vec<(NaiveDate, u16)> {
        if area_height == 0 {
            return Vec::new();
        }

        // First frame: pick a scroll_top that places `selected` near the bottom.
        // We aim for one card visible below the selected (the padding row).
        if !self.initialized {
            self.scroll_top = self.selected_date - Duration::days(2);
            self.initialized = true;
        }

        // Maintain padding-of-1 above and below the selected card.
        // Iteratively shift scroll_top until selected has cards on both sides
        // fitting in the area.
        for _ in 0..32 {
            let plan = self.plan_box_from(self.scroll_top, area_height, store);
            let sel_pos = plan
                .iter()
                .position(|(d, _)| *d == self.selected_date);
            match sel_pos {
                None => {
                    // selected not visible at all
                    if self.selected_date < self.scroll_top {
                        self.scroll_top = self.selected_date - Duration::days(1);
                    } else {
                        // Push scroll_top forward so selected becomes visible.
                        self.scroll_top += Duration::days(1);
                    }
                }
                Some(0) if plan.len() > 1 => {
                    // selected on top, need padding above
                    self.scroll_top -= Duration::days(1);
                }
                Some(i) if i == plan.len() - 1 && plan.len() > 1 => {
                    // selected on bottom, need padding below
                    self.scroll_top += Duration::days(1);
                }
                Some(_) => return plan,
            }
        }
        // Fallback: just render whatever fits.
        self.plan_box_from(self.scroll_top, area_height, store)
    }

    fn plan_box_from(
        &self,
        start: NaiveDate,
        area_height: u16,
        store: &Store,
    ) -> Vec<(NaiveDate, u16)> {
        let mut out = Vec::new();
        let mut used = 0u16;
        let mut date = start;
        while used < area_height {
            let h = card_height(store, date);
            if used + h > area_height {
                break;
            }
            out.push((date, h));
            used += h;
            date += Duration::days(1);
        }
        out
    }

    fn draw_card(&self, f: &mut Frame, area: Rect, date: NaiveDate, store: &Store) {
        let is_selected = date == self.selected_date;
        let is_today = date == self.today;

        let border_color = if is_selected {
            Color::Yellow
        } else if is_today {
            Color::Green
        } else {
            Color::DarkGray
        };

        let date_label = format_date_short(date);
        let title_text = if is_today {
            format!(" {date_label} (today) ")
        } else {
            format!(" {date_label} ")
        };
        let mut title_style = Style::default().fg(border_color);
        if is_selected {
            title_style = title_style.add_modifier(Modifier::BOLD);
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(title_text.clone(), title_style));
        let inner = block.inner(area);
        f.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(inner);

        let entry = store.get(date);
        let dim = Style::default().fg(Color::DarkGray);

        let planned_content: Vec<Line> = match entry {
            Some(e) if !e.planning.is_empty() => bullet_lines(&e.planning),
            _ => vec![Line::from(Span::styled(" —", dim))],
        };
        let did_content: Vec<Line> = match entry {
            Some(e) if !e.did.is_empty() => bullet_lines(&e.did),
            _ => vec![Line::from(Span::styled(" —", dim))],
        };

        let planned_widget = Paragraph::new(planned_content)
            .block(
                Block::default()
                    .borders(Borders::RIGHT)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .wrap(Wrap { trim: false });
        let did_widget = Paragraph::new(did_content).wrap(Wrap { trim: false });

        f.render_widget(planned_widget, cols[0]);
        f.render_widget(did_widget, cols[1]);

        // Inline "Planned" / "Did" headings on the top border, plus ┬/┴
        // connectors where the inner column separator meets the borders.
        // Skip if the card isn't wide enough to fit everything cleanly.
        let title_end = area.x + 1 + title_text.chars().count() as u16;
        let planned_label = " Planned ";
        let did_label = " Did ";
        let label_style = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD);
        let sep_x = cols[0].x + cols[0].width - 1;
        let planned_x = label_centered(planned_label, cols[0]);
        let did_x = label_centered(did_label, cols[1]);
        let buf = f.buffer_mut();
        if planned_x >= title_end + 1 {
            buf.set_string(planned_x, area.y, planned_label, label_style);
        }
        if did_x + did_label.chars().count() as u16 + 1 <= area.x + area.width {
            buf.set_string(did_x, area.y, did_label, label_style);
        }
        // Border connectors at the top and bottom of the inner separator.
        if sep_x >= area.x + 1 && sep_x + 1 < area.x + area.width {
            buf.set_string(
                sep_x,
                area.y,
                "┬",
                Style::default().fg(Color::DarkGray),
            );
            buf.set_string(
                sep_x,
                area.y + area.height - 1,
                "┴",
                Style::default().fg(Color::DarkGray),
            );
        }
    }

    // ---------------- Scroll math (compact view) ----------------

    fn ensure_visible_rows(&mut self, n: i64) {
        if n <= 0 {
            return;
        }
        if !self.initialized {
            let target_row = if n >= 3 { n - 2 } else { (n - 1).max(0) };
            self.scroll_top = self.selected_date - Duration::days(target_row);
            self.initialized = true;
            return;
        }
        let row = (self.selected_date - self.scroll_top).num_days();
        if n >= 3 {
            if row < 1 {
                self.scroll_top = self.selected_date - Duration::days(1);
            } else if row > n - 2 {
                self.scroll_top = self.selected_date - Duration::days(n - 2);
            }
        } else {
            if row < 0 {
                self.scroll_top = self.selected_date;
            } else if row > n - 1 {
                self.scroll_top = self.selected_date - Duration::days(n - 1);
            }
        }
    }
}

fn card_height(store: &Store, date: NaiveDate) -> u16 {
    let entry = store.get(date);
    let content_lines = entry
        .map(|e| e.planning.len().max(e.did.len()))
        .unwrap_or(0)
        .max(1);
    // 1 (top border, with inline headings) + content + 1 (bottom border)
    (2 + content_lines) as u16
}

fn label_centered(label: &str, col: Rect) -> u16 {
    let len = label.chars().count() as u16;
    if len >= col.width {
        col.x
    } else {
        col.x + (col.width - len) / 2
    }
}

const RELNUM_WIDTH: u16 = 5;

fn bullet_lines(items: &[String]) -> Vec<Line<'static>> {
    items
        .iter()
        .map(|b| Line::from(format!(" - {b}")))
        .collect()
}

fn preview_join(store: &Store, date: NaiveDate) -> (String, String) {
    match store.get(date) {
        Some(e) => (e.planning.join(" · "), e.did.join(" · ")),
        None => (String::new(), String::new()),
    }
}

fn empty_or(s: String, placeholder: &str, dim: Style) -> Cell<'static> {
    if s.is_empty() {
        Cell::from(Span::styled(placeholder.to_string(), dim))
    } else {
        Cell::from(s)
    }
}

fn relnum_label(date: NaiveDate, selected: NaiveDate) -> String {
    let delta = (date - selected).num_days();
    if delta == 0 {
        "  0 ".to_string()
    } else {
        format!("{:>3} ", delta.unsigned_abs())
    }
}

fn format_date_short(d: NaiveDate) -> String {
    d.format("%a %b %-d").to_string()
}
