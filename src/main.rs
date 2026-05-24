mod app;
mod clipboard;
mod format;
mod help;
mod history;
mod storage;
mod vim;

use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::{
    event::{
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::app::App;

fn main() -> Result<()> {
    let path = storage::default_path()?;
    let mut app = App::new(path)?;

    let (mut terminal, kbd_enhanced) = setup_terminal()?;
    let result = app.run(&mut terminal);
    restore_terminal(&mut terminal, kbd_enhanced)?;
    result
}

fn setup_terminal() -> Result<(Terminal<CrosstermBackend<Stdout>>, bool)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Kitty keyboard protocol lets terminals like Ghostty / kitty / WezTerm
    // report distinct events for Ctrl+Backspace, Shift+Enter, etc.
    let kbd_enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if kbd_enhanced {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS,
            ),
        )?;
    }

    let backend = CrosstermBackend::new(stdout);
    Ok((Terminal::new(backend)?, kbd_enhanced))
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    kbd_enhanced: bool,
) -> Result<()> {
    if kbd_enhanced {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
