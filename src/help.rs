use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

pub fn draw(f: &mut Frame, parent: Rect) {
    let area = centered_rect(80, 80, parent);
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " Keybinds ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let body = Paragraph::new(content()).wrap(Wrap { trim: false });
    f.render_widget(body, inner);
}

fn content() -> Vec<Line<'static>> {
    let h = |s: &'static str| {
        Line::from(Span::styled(
            s,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
    };
    let row = |key: &'static str, desc: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {key:<14}"), Style::default().fg(Color::Cyan)),
            Span::raw(desc),
        ])
    };

    vec![
        h("Today screen — normal mode"),
        row("i  a  o", "insert at cursor / after / open line below"),
        row("I  A  O", "insert at line start / line end / open line above"),
        row("h j k l", "move by char / line"),
        row("w  b  e", "next/prev word, word end"),
        row("0  $", "line start / end"),
        row("gg  G", "buffer top / bottom"),
        row("x", "delete char under cursor"),
        row("dd  cc", "delete / change line"),
        row("D  C", "delete / change to end of line"),
        row("u  Ctrl-R", "undo / redo"),
        row("Tab", "switch pane (yesterday-did ↔ today-planning)"),
        row("</>", "previous / next day"),
        row("q", "quit (saves)"),
        Line::from(""),
        h("Today screen — insert mode"),
        row("Esc  jj", "back to normal mode"),
        row("Ctrl-Bksp", "delete previous word"),
        row("Tab", "switch pane"),
        Line::from(""),
        h("Yank (press y then target)"),
        row("yt", "Teams (HTML, paste into Teams)"),
        row("yx", "Xero (plain newline-separated)"),
        row("yd", "yesterday's did (bulleted)"),
        row("yp", "today's planning (bulleted)"),
        Line::from(""),
        h("Ex commands"),
        row(":w", "save"),
        row(":wq  :x", "save and quit"),
        row(":q", "quit"),
        Line::from(""),
        h("Leader (Space then key)"),
        row("<Space>h", "open history"),
        row("<Space>s", "open settings"),
        row("<Space>?", "open this help"),
        Line::from(""),
        h("Settings screen"),
        row("j  k", "move between items / fields"),
        row("l Enter", "enter submenu / edit text / toggle"),
        row("h  Esc", "back to previous level (Esc at top saves+closes)"),
        row("Space", "toggle bool / enter edit on text"),
        row("Esc/Enter", "exit edit mode (in edit)"),
        row(":w", "save without closing"),
        row(":wq  :x", "save and close"),
        row(":q", "close without saving"),
        Line::from(""),
        h("History screen"),
        row("j  k", "move one day"),
        row("<n>j  <n>k", "move n days (multi-digit, e.g. 22j)"),
        row("gg", "jump to earliest recorded day"),
        row("G", "jump to today"),
        row("z", "toggle box / compact view"),
        row("Enter", "select day (lands as yesterday's did)"),
        row("Esc  q", "close history"),
        Line::from(""),
        h("Anywhere"),
        row("?", "open this help"),
        row("Ctrl-Q", "quit (saves)"),
        Line::from(""),
        Line::from(Span::styled(
            "  Press any key to dismiss.",
            Style::default().fg(Color::DarkGray),
        )),
    ]
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}
