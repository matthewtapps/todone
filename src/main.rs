mod clipboard;
mod format;
mod storage;

fn main() -> anyhow::Result<()> {
    // TUI comes next. For now, the modules are exercised by their unit tests.
    let path = storage::default_path()?;
    let store = storage::load(&path)?;
    println!("loaded {} entries from {}", store.entries.len(), path.display());
    Ok(())
}
