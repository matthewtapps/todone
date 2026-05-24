use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers as M};
use tui_textarea::{CursorMove, TextArea};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Insert,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Pending {
    None,
    G,
    D,
    C,
}

/// Max gap between two `j` presses for them to be treated as `<Esc>` in insert mode.
const JJ_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: (usize, usize),
}

pub struct VimBuffer<'a> {
    pub area: TextArea<'a>,
    pub mode: Mode,
    pending: Pending,
    last_j_at: Option<Instant>,
    pub jj_timeout: Duration,
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    /// Snapshot taken when entering insert mode; committed to undo on Esc.
    pre_insert: Option<Snapshot>,
}

impl<'a> VimBuffer<'a> {
    pub fn new(area: TextArea<'a>) -> Self {
        Self {
            area,
            mode: Mode::Normal,
            pending: Pending::None,
            last_j_at: None,
            jj_timeout: JJ_TIMEOUT,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pre_insert: None,
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.area.lines().to_vec(),
            cursor: self.area.cursor(),
        }
    }

    fn restore(&mut self, s: Snapshot) {
        let mut area = if s.lines.is_empty() {
            TextArea::default()
        } else {
            TextArea::new(s.lines)
        };
        area.move_cursor(CursorMove::Jump(s.cursor.0 as u16, s.cursor.1 as u16));
        self.area = area;
    }

    /// Push `prev` to the undo stack if the current buffer differs from it.
    fn commit(&mut self, prev: Snapshot) {
        if prev.lines != self.area.lines() {
            self.undo_stack.push(prev);
            self.redo_stack.clear();
        }
    }

    fn undo(&mut self) {
        if let Some(snap) = self.undo_stack.pop() {
            let current = self.snapshot();
            self.restore(snap);
            self.redo_stack.push(current);
        }
    }

    fn redo(&mut self) {
        if let Some(snap) = self.redo_stack.pop() {
            let current = self.snapshot();
            self.restore(snap);
            self.undo_stack.push(current);
        }
    }

    fn enter_insert(&mut self) {
        if self.pre_insert.is_none() {
            self.pre_insert = Some(self.snapshot());
        }
        self.mode = Mode::Insert;
    }

    fn leave_insert(&mut self) {
        self.mode = Mode::Normal;
        self.last_j_at = None;
        if let Some(snap) = self.pre_insert.take() {
            self.commit(snap);
        }
    }

    pub fn lines(&self) -> &[String] {
        self.area.lines()
    }

    /// Returns true if the key was fully consumed. Returns false if the key
    /// should be handled at the app level (e.g. `y` yank verb, `<space>` leader).
    pub fn input(&mut self, k: KeyEvent) -> bool {
        // Ctrl+Backspace works in any mode.
        if k.code == KeyCode::Backspace && k.modifiers.contains(M::CONTROL) {
            self.area.delete_word();
            return true;
        }
        match self.mode {
            Mode::Insert => {
                self.input_insert(k);
                true
            }
            Mode::Normal => self.input_normal(k),
        }
    }

    fn input_insert(&mut self, k: KeyEvent) {
        if k.code == KeyCode::Esc {
            self.leave_insert();
            return;
        }
        // Quick `jj` escape: only triggers when the two j's arrive within JJ_TIMEOUT.
        // Pause briefly between them to type a literal `jj`.
        if k.code == KeyCode::Char('j') && k.modifiers.is_empty() {
            if let Some(t) = self.last_j_at {
                if t.elapsed() <= self.jj_timeout {
                    self.area.delete_char();
                    self.leave_insert();
                    return;
                }
            }
            self.area.input(k);
            self.last_j_at = Some(Instant::now());
            return;
        }
        self.last_j_at = None;
        self.area.input(k);
    }

    /// Normal-mode dispatch. Returns false if the app should handle the key.
    fn input_normal(&mut self, k: KeyEvent) -> bool {
        // Resolve pending two-char sequences first.
        match self.pending {
            Pending::G => {
                self.pending = Pending::None;
                if k.code == KeyCode::Char('g') {
                    self.area.move_cursor(CursorMove::Top);
                    self.area.move_cursor(CursorMove::Head);
                }
                return true;
            }
            Pending::D => {
                self.pending = Pending::None;
                if k.code == KeyCode::Char('d') {
                    let snap = self.snapshot();
                    delete_line(&mut self.area);
                    self.commit(snap);
                }
                return true;
            }
            Pending::C => {
                self.pending = Pending::None;
                if k.code == KeyCode::Char('c') {
                    self.enter_insert();
                    change_line(&mut self.area);
                }
                return true;
            }
            Pending::None => {}
        }

        // App-level verbs: defer to caller (q=quit, y=yank, space=leader, : ex, </> nav, ?=help).
        if k.modifiers.is_empty() || k.modifiers == M::SHIFT {
            if matches!(
                k.code,
                KeyCode::Char('q')
                    | KeyCode::Char('y')
                    | KeyCode::Char(' ')
                    | KeyCode::Char(':')
                    | KeyCode::Char('<')
                    | KeyCode::Char('>')
                    | KeyCode::Char('[')
                    | KeyCode::Char(']')
                    | KeyCode::Char('?')
            ) {
                return false;
            }
        }

        let plain = k.modifiers.is_empty() || k.modifiers == M::SHIFT;
        if plain {
            match k.code {
                // Movement
                KeyCode::Char('h') => self.area.move_cursor(CursorMove::Back),
                KeyCode::Char('j') => self.area.move_cursor(CursorMove::Down),
                KeyCode::Char('k') => self.area.move_cursor(CursorMove::Up),
                KeyCode::Char('l') => self.area.move_cursor(CursorMove::Forward),
                KeyCode::Char('w') => self.area.move_cursor(CursorMove::WordForward),
                KeyCode::Char('b') => self.area.move_cursor(CursorMove::WordBack),
                KeyCode::Char('e') => self.area.move_cursor(CursorMove::WordEnd),
                KeyCode::Char('0') => self.area.move_cursor(CursorMove::Head),
                KeyCode::Char('$') => self.area.move_cursor(CursorMove::End),
                KeyCode::Char('G') => {
                    self.area.move_cursor(CursorMove::Bottom);
                    self.area.move_cursor(CursorMove::End);
                }

                // Enter insert
                KeyCode::Char('i') => self.enter_insert(),
                KeyCode::Char('I') => {
                    self.enter_insert();
                    self.area.move_cursor(CursorMove::Head);
                }
                KeyCode::Char('a') => {
                    self.enter_insert();
                    self.area.move_cursor(CursorMove::Forward);
                }
                KeyCode::Char('A') => {
                    self.enter_insert();
                    self.area.move_cursor(CursorMove::End);
                }
                KeyCode::Char('o') => {
                    self.enter_insert();
                    self.area.move_cursor(CursorMove::End);
                    self.area.insert_newline();
                }
                KeyCode::Char('O') => {
                    self.enter_insert();
                    self.area.move_cursor(CursorMove::Head);
                    self.area.insert_newline();
                    self.area.move_cursor(CursorMove::Up);
                }

                // Delete / change
                KeyCode::Char('x') => {
                    let snap = self.snapshot();
                    self.area.delete_next_char();
                    self.commit(snap);
                }
                KeyCode::Char('D') => {
                    let snap = self.snapshot();
                    self.area.delete_line_by_end();
                    self.commit(snap);
                }
                KeyCode::Char('C') => {
                    self.enter_insert();
                    self.area.delete_line_by_end();
                }

                // Undo
                KeyCode::Char('u') => {
                    self.undo();
                }

                // Pending sequences
                KeyCode::Char('g') => self.pending = Pending::G,
                KeyCode::Char('d') => self.pending = Pending::D,
                KeyCode::Char('c') => self.pending = Pending::C,

                _ => {}
            }
            return true;
        }

        if k.modifiers.contains(M::CONTROL) {
            if let KeyCode::Char('r') = k.code {
                self.redo();
            }
            return true;
        }
        true
    }

    pub fn has_pending(&self) -> bool {
        self.pending != Pending::None
    }
}

fn delete_line(area: &mut TextArea) {
    let (row, _) = area.cursor();
    let total = area.lines().len();
    if total <= 1 {
        // Sole line: clear its content, the line itself must remain.
        area.move_cursor(CursorMove::Head);
        area.delete_line_by_end();
        return;
    }
    if row + 1 < total {
        // Not the last line: select [head, head-of-next] — drops content + trailing newline.
        area.move_cursor(CursorMove::Head);
        area.start_selection();
        area.move_cursor(CursorMove::Down);
        area.move_cursor(CursorMove::Head);
        area.cut();
    } else {
        // Last line of multi-line buffer: select [end-of-prev, end-of-current]
        // — drops the leading newline + current line content.
        area.move_cursor(CursorMove::Up);
        area.move_cursor(CursorMove::End);
        area.start_selection();
        area.move_cursor(CursorMove::Down);
        area.move_cursor(CursorMove::End);
        area.cut();
    }
}

/// `cc`: clear the line content but keep the (now empty) line in place.
fn change_line(area: &mut TextArea) {
    area.move_cursor(CursorMove::Head);
    area.delete_line_by_end();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn shift(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    fn buf(lines: &[&str]) -> VimBuffer<'static> {
        let owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        VimBuffer::new(TextArea::new(owned))
    }

    fn send(b: &mut VimBuffer<'_>, keys: &[KeyEvent]) {
        for k in keys {
            b.input(*k);
        }
    }

    #[test]
    fn starts_in_normal_mode() {
        let b = buf(&["hi"]);
        assert_eq!(b.mode, Mode::Normal);
    }

    #[test]
    fn i_enters_insert_esc_returns_to_normal() {
        let mut b = buf(&["hi"]);
        send(&mut b, &[key('i')]);
        assert_eq!(b.mode, Mode::Insert);
        send(&mut b, &[esc()]);
        assert_eq!(b.mode, Mode::Normal);
    }

    #[test]
    fn insert_mode_text_is_inserted() {
        let mut b = buf(&[""]);
        send(&mut b, &[key('i'), key('a'), key('b'), key('c')]);
        assert_eq!(b.lines(), &["abc".to_string()]);
    }

    #[test]
    fn normal_mode_hjkl_does_not_insert_text() {
        let mut b = buf(&["abc", "def"]);
        send(&mut b, &[key('j'), key('l'), key('h'), key('k')]);
        assert_eq!(b.lines(), &["abc".to_string(), "def".to_string()]);
    }

    #[test]
    fn dd_deletes_current_line() {
        let mut b = buf(&["one", "two", "three"]);
        // cursor starts at (0,0). dd should remove "one".
        send(&mut b, &[key('d'), key('d')]);
        assert_eq!(b.lines(), &["two".to_string(), "three".to_string()]);
    }

    #[test]
    fn dd_on_sole_line_clears_it() {
        let mut b = buf(&["only"]);
        send(&mut b, &[key('d'), key('d')]);
        assert_eq!(b.lines(), &["".to_string()]);
    }

    #[test]
    fn dd_on_last_line_of_multi_removes_the_line() {
        let mut b = buf(&["one", "two"]);
        // Move to last line, then dd.
        send(&mut b, &[shift('G'), key('d'), key('d')]);
        assert_eq!(b.lines(), &["one".to_string()]);
    }

    #[test]
    fn dd_on_trailing_blank_line_removes_it() {
        let mut b = buf(&["bullet", ""]);
        send(&mut b, &[shift('G'), key('d'), key('d')]);
        assert_eq!(b.lines(), &["bullet".to_string()]);
    }

    #[test]
    fn cc_deletes_line_and_enters_insert() {
        let mut b = buf(&["one", "two"]);
        send(&mut b, &[key('c'), key('c')]);
        assert_eq!(b.mode, Mode::Insert);
        send(&mut b, &[key('x')]); // would be cmd in normal, text in insert
        assert_eq!(b.lines()[0], "x");
    }

    #[test]
    fn o_opens_line_below() {
        let mut b = buf(&["one", "two"]);
        send(&mut b, &[key('o'), key('X')]);
        assert_eq!(b.lines(), &["one".to_string(), "X".to_string(), "two".to_string()]);
    }

    #[test]
    fn capital_o_opens_line_above() {
        let mut b = buf(&["one", "two"]);
        send(&mut b, &[shift('O'), key('X')]);
        assert_eq!(b.lines(), &["X".to_string(), "one".to_string(), "two".to_string()]);
    }

    #[test]
    fn gg_jumps_to_top() {
        let mut b = buf(&["one", "two", "three"]);
        send(&mut b, &[shift('G'), key('g'), key('g'), key('i'), key('!')]);
        assert_eq!(b.lines()[0], "!one");
    }

    #[test]
    fn capital_g_jumps_to_bottom() {
        let mut b = buf(&["one", "two", "three"]);
        send(&mut b, &[shift('G'), key('a'), key('!')]);
        assert_eq!(b.lines()[2], "three!");
    }

    #[test]
    fn x_deletes_char_under_cursor() {
        let mut b = buf(&["hello"]);
        send(&mut b, &[key('x')]);
        assert_eq!(b.lines()[0], "ello");
    }

    #[test]
    fn undo_treats_insert_session_as_one_block() {
        let mut b = buf(&[""]);
        send(&mut b, &[key('i'), key('a'), key('b'), key('c'), esc()]);
        assert_eq!(b.lines()[0], "abc");
        send(&mut b, &[key('u')]);
        assert_eq!(b.lines()[0], ""); // single undo wipes the whole "abc"
        send(&mut b, &[ctrl('r')]);
        assert_eq!(b.lines()[0], "abc");
    }

    #[test]
    fn undo_skips_net_zero_insert_sessions() {
        // jj-escape inserts then deletes a j; net change is none, so no undo step.
        let mut b = buf(&["start"]);
        send(&mut b, &[key('A'), key('!'), esc()]); // append "!" → "start!"
        send(&mut b, &[key('i'), key('j'), key('j')]); // jj-escape, no change
        assert_eq!(b.lines()[0], "start!");
        send(&mut b, &[key('u')]);
        // Only one undo step exists (for the "!" insert), and it should clear that.
        assert_eq!(b.lines()[0], "start");
    }

    #[test]
    fn undo_normal_mode_edits_each_one_step() {
        let mut b = buf(&["abc"]);
        send(&mut b, &[key('x')]); // "bc"
        send(&mut b, &[key('x')]); // "c"
        assert_eq!(b.lines()[0], "c");
        send(&mut b, &[key('u')]);
        assert_eq!(b.lines()[0], "bc");
        send(&mut b, &[key('u')]);
        assert_eq!(b.lines()[0], "abc");
    }

    #[test]
    fn redo_is_cleared_by_new_edit() {
        let mut b = buf(&["abc"]);
        send(&mut b, &[key('x')]); // "bc"
        send(&mut b, &[key('u')]); // back to "abc"
        send(&mut b, &[key('x')]); // "bc" again — should clear redo
        send(&mut b, &[ctrl('r')]); // nothing to redo
        assert_eq!(b.lines()[0], "bc");
    }

    #[test]
    fn ctrl_backspace_works_in_insert_mode() {
        let mut b = buf(&[""]);
        send(&mut b, &[key('i'), key('h'), key('e'), key('l'), key('l'), key('o')]);
        let bk = KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL);
        b.input(bk);
        assert_eq!(b.lines()[0], "");
    }

    #[test]
    fn y_and_space_are_not_consumed_in_normal_mode() {
        let mut b = buf(&["hi"]);
        assert!(!b.input(key('y')));
        assert!(!b.input(key(' ')));
    }

    #[test]
    fn y_in_insert_mode_is_typed() {
        let mut b = buf(&[""]);
        send(&mut b, &[key('i')]);
        assert!(b.input(key('y')));
        assert_eq!(b.lines()[0], "y");
    }

    #[test]
    fn jj_in_insert_escapes_to_normal() {
        let mut b = buf(&[""]);
        send(&mut b, &[key('i'), key('a'), key('b'), key('j'), key('j')]);
        assert_eq!(b.mode, Mode::Normal);
        assert_eq!(b.lines()[0], "ab");
    }

    #[test]
    fn single_j_in_insert_is_typed_normally() {
        let mut b = buf(&[""]);
        send(&mut b, &[key('i'), key('j'), key('o'), key('e')]);
        assert_eq!(b.mode, Mode::Insert);
        assert_eq!(b.lines()[0], "joe");
    }

    #[test]
    fn jj_with_other_keys_between_does_not_escape() {
        let mut b = buf(&[""]);
        send(&mut b, &[key('i'), key('j'), key('x'), key('j')]);
        assert_eq!(b.mode, Mode::Insert);
        assert_eq!(b.lines()[0], "jxj");
    }
}
