mod app;
mod calendar;
mod clipboard;
mod config;
mod context;
mod format;
mod gitlab;
mod help;
mod history;
mod settings;
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
    // Multi-thread runtime so a slow GitLab fetch doesn't stall others.
    // Two workers is plenty for a handful of concurrent HTTP requests.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;

    let path = storage::default_path()?;
    let mut app = App::new(path, runtime.handle().clone())?;

    let (mut terminal, kbd_enhanced) = setup_terminal()?;
    let result = app.run(&mut terminal);
    restore_terminal(&mut terminal, kbd_enhanced)?;
    // Drop the runtime explicitly so in-flight tasks get cancelled before exit.
    drop(runtime);
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
