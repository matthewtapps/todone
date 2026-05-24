use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    #[serde(default)]
    pub gitlab: GitlabConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitlabConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub instance_url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub username: String,
}

pub fn default_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("could not determine XDG config dir")?
        .join("standup");
    Ok(dir.join("config.toml"))
}

pub fn load(path: &Path) -> Result<Settings> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    let data = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    if data.trim().is_empty() {
        return Ok(Settings::default());
    }
    toml::from_str(&data).with_context(|| format!("parsing {}", path.display()))
}

/// Atomic write at 0600 perms — the file holds a PAT.
pub fn save(path: &Path, s: &Settings) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    let data = toml::to_string_pretty(s)?;
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "standup-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn missing_file_returns_default() {
        let p = tempdir().join("config.toml");
        assert_eq!(load(&p).unwrap(), Settings::default());
    }

    #[test]
    fn roundtrip_preserves_fields() {
        let p = tempdir().join("config.toml");
        let s = Settings {
            gitlab: GitlabConfig {
                enabled: true,
                instance_url: "https://gitlab.example.com".into(),
                token: "glpat-abc".into(),
                username: "mtapps".into(),
            },
        };
        save(&p, &s).unwrap();
        assert_eq!(load(&p).unwrap(), s);
    }

    #[test]
    fn save_creates_file_with_0600_perms() {
        use std::os::unix::fs::PermissionsExt;
        let p = tempdir().join("config.toml");
        save(&p, &Settings::default()).unwrap();
        let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
