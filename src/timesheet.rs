//! Timesheet documents on disk, shared by the web server and the CLI.

use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use serde_json::{Map, Value};

pub fn dir() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context("could not determine XDG data dir")?
        .join("todone")
        .join("timesheet"))
}

pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

pub fn month_path(dir: &Path, month: &str) -> Result<PathBuf> {
    let valid =
        month.len() == 7 && NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").is_ok();
    if !valid {
        bail!("month must be YYYY-MM, got {month:?}");
    }
    Ok(dir.join(format!("{month}.json")))
}

/// The version and the document without it; a missing file is version 0 and empty.
pub fn read(path: &Path) -> Result<(u64, Map<String, Value>)> {
    let data = match fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok((0, Map::new())),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut doc: Map<String, Value> =
        serde_json::from_str(&data).with_context(|| format!("parsing {}", path.display()))?;
    let version = doc.remove("version").and_then(|v| v.as_u64()).unwrap_or(0);
    Ok((version, doc))
}

pub enum Put {
    Saved(u64),
    Stale(u64, Map<String, Value>),
}

/// Saves `doc` only while the file is still at `expected`.
pub fn put(path: &Path, expected: u64, doc: Map<String, Value>) -> Result<Put> {
    with_lock(path, || {
        let (version, current) = read(path)?;
        if version != expected {
            return Ok(Put::Stale(version, current));
        }
        write(path, version + 1, doc)?;
        Ok(Put::Saved(version + 1))
    })
}

pub fn modify(path: &Path, f: impl FnOnce(&mut Map<String, Value>) -> Result<()>) -> Result<u64> {
    with_lock(path, || {
        let (version, mut doc) = read(path)?;
        f(&mut doc)?;
        write(path, version + 1, doc)?;
        Ok(version + 1)
    })
}

/// Writes only when `f` returns true; otherwise the version stays put.
pub fn modify_if(
    path: &Path,
    f: impl FnOnce(&mut Map<String, Value>) -> Result<bool>,
) -> Result<()> {
    with_lock(path, || {
        let (version, mut doc) = read(path)?;
        if f(&mut doc)? {
            write(path, version + 1, doc)?;
        }
        Ok(())
    })
}

// The lock lives in a sidecar file because `write` renames over the document.
fn with_lock<T>(path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let parent = path.parent().context("timesheet path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let lock_path = path.with_extension("json.lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("locking {}", lock_path.display()))?;
    f()
}

fn write(path: &Path, version: u64, mut doc: Map<String, Value>) -> Result<()> {
    doc.insert("version".into(), version.into());
    let tmp = path.with_extension("json.tmp");
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(serde_json::to_string_pretty(&doc)?.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "todone-timesheet-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn doc(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn missing_file_is_version_zero_and_empty() {
        let p = tempdir().join("2026-10.json");
        assert_eq!(read(&p).unwrap(), (0, Map::new()));
    }

    #[test]
    fn put_with_current_version_saves_and_bumps() {
        let p = tempdir().join("2026-10.json");
        assert!(matches!(
            put(&p, 0, doc(json!({"rows": {}}))).unwrap(),
            Put::Saved(1)
        ));
        assert!(matches!(
            put(&p, 1, doc(json!({"rows": {"a": 1}}))).unwrap(),
            Put::Saved(2)
        ));
        assert_eq!(read(&p).unwrap(), (2, doc(json!({"rows": {"a": 1}}))));
    }

    #[test]
    fn stale_put_returns_disk_contents_and_writes_nothing() {
        let p = tempdir().join("2026-10.json");
        put(&p, 0, doc(json!({"rows": {"disk": 1}}))).unwrap();
        let before = fs::read_to_string(&p).unwrap();
        match put(&p, 0, doc(json!({"rows": {"stale": 1}}))).unwrap() {
            Put::Stale(v, d) => assert_eq!((v, d), (1, doc(json!({"rows": {"disk": 1}})))),
            Put::Saved(_) => panic!("stale write was saved"),
        }
        assert_eq!(fs::read_to_string(&p).unwrap(), before);
    }

    #[test]
    fn unparseable_file_errors_and_is_not_overwritten() {
        let p = tempdir().join("2026-10.json");
        fs::write(&p, "{ not json").unwrap();
        assert!(read(&p).is_err());
        assert!(put(&p, 0, Map::new()).is_err());
        assert!(modify(&p, |_| Ok(())).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "{ not json");
    }

    #[test]
    fn leftover_tmp_file_does_not_block_writes() {
        let p = tempdir().join("2026-10.json");
        fs::write(p.with_extension("json.tmp"), "junk from a crash").unwrap();
        assert!(matches!(
            put(&p, 0, doc(json!({"rows": {}}))).unwrap(),
            Put::Saved(1)
        ));
    }

    #[test]
    fn modify_applies_and_bumps_version() {
        let p = tempdir().join("settings.json");
        assert_eq!(
            modify(&p, |d| {
                d.insert("shown".into(), json!(["t"]));
                Ok(())
            })
            .unwrap(),
            1
        );
        assert_eq!(read(&p).unwrap(), (1, doc(json!({"shown": ["t"]}))));
    }

    #[test]
    fn month_path_accepts_only_yyyy_mm() {
        let d = tempdir();
        assert!(month_path(&d, "2026-10").is_ok());
        for bad in ["2026-13", "26-10", "2026-1", "../etc", "2026-10.json"] {
            assert!(month_path(&d, bad).is_err(), "{bad}");
        }
    }
}
