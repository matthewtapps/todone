use anyhow::{Context, Result};
use arboard::Clipboard as Arboard;

/// Thin wrapper around `arboard::Clipboard`. Held long-lived by `App` because
/// on Linux/Wayland the clipboard owner must stay alive to serve paste
/// requests — once this is dropped, paste targets see empty content.
pub struct Clipboard {
    inner: Arboard,
}

impl Clipboard {
    pub fn new() -> Result<Self> {
        let inner = Arboard::new().context("initialising system clipboard")?;
        Ok(Self { inner })
    }

    pub fn copy(&mut self, text: &str) -> Result<()> {
        self.inner.set_text(text.to_owned()).context("writing clipboard text")
    }

    /// Place HTML on the clipboard so apps like Teams paste it as rich text.
    /// The plain stripped text is offered as a fallback for plain-only consumers.
    pub fn copy_html(&mut self, html: &str, plain_alt: &str) -> Result<()> {
        self.inner
            .set_html(html.to_owned(), Some(plain_alt.to_owned()))
            .context("writing clipboard html")
    }
}
