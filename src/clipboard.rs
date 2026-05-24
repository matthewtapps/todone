use std::{io::Write, process::{Command, Stdio}};

use anyhow::{Context, Result};

/// Pipe `text` into `wl-copy`. Returns an error if `wl-copy` is missing or fails.
pub fn copy(text: &str) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning wl-copy (is wl-clipboard installed?)")?;

    {
        let stdin = child.stdin.as_mut().context("wl-copy stdin")?;
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait().context("waiting for wl-copy")?;
    if !status.success() {
        anyhow::bail!("wl-copy exited with status {status}");
    }
    Ok(())
}
