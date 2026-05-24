use std::{io::Write, process::{Command, Stdio}};

use anyhow::{Context, Result};

/// Pipe `text` into `wl-copy` as plain text.
pub fn copy(text: &str) -> Result<()> {
    copy_with_mime(text, None)
}

/// Pipe `text` into `wl-copy` declaring it as the given MIME type so apps
/// like Teams will paste it as rich content (e.g. `text/html`).
pub fn copy_html(text: &str) -> Result<()> {
    copy_with_mime(text, Some("text/html"))
}

fn copy_with_mime(text: &str, mime: Option<&str>) -> Result<()> {
    let mut cmd = Command::new("wl-copy");
    if let Some(m) = mime {
        cmd.arg("-t").arg(m);
    }
    let mut child = cmd
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
