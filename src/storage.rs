use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    #[serde(default)]
    pub did: Vec<String>,
    #[serde(default)]
    pub planning: Vec<String>,
}

impl Entry {
    pub fn is_empty(&self) -> bool {
        self.did.is_empty() && self.planning.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Store {
    #[serde(default)]
    pub entries: BTreeMap<NaiveDate, Entry>,
}

impl Store {
    pub fn get(&self, date: NaiveDate) -> Option<&Entry> {
        self.entries.get(&date)
    }

    pub fn entry_mut(&mut self, date: NaiveDate) -> &mut Entry {
        self.entries.entry(date).or_default()
    }

    pub fn remove(&mut self, date: NaiveDate) -> Option<Entry> {
        self.entries.remove(&date)
    }

    /// Dates with any content, newest first.
    pub fn dates_desc(&self) -> Vec<NaiveDate> {
        self.entries
            .iter()
            .filter(|(_, e)| !e.is_empty())
            .map(|(d, _)| *d)
            .rev()
            .collect()
    }
}

pub fn default_path() -> Result<PathBuf> {
    let dir = dirs::data_dir()
        .context("could not determine XDG data dir")?
        .join("standup");
    Ok(dir.join("entries.json"))
}

pub fn load(path: &Path) -> Result<Store> {
    if !path.exists() {
        return Ok(Store::default());
    }
    let data = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    if data.trim().is_empty() {
        return Ok(Store::default());
    }
    serde_json::from_str(&data)
        .with_context(|| format!("parsing {}", path.display()))
}

pub fn save(path: &Path, store: &Store) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(store)?;
    {
        let mut f = fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Previous *work* day. Mon -> Fri, Sun -> Fri, Sat -> Fri, else day - 1.
pub fn previous_workday(d: NaiveDate) -> NaiveDate {
    let back = match d.weekday() {
        Weekday::Mon => 3,
        Weekday::Sun => 2,
        Weekday::Sat => 1,
        _ => 1,
    };
    d - Duration::days(back)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn workday_monday_jumps_to_friday() {
        // 2026-05-25 is a Monday.
        assert_eq!(previous_workday(d("2026-05-25")), d("2026-05-22"));
    }

    #[test]
    fn workday_tuesday_to_monday() {
        // 2026-05-26 is a Tuesday.
        assert_eq!(previous_workday(d("2026-05-26")), d("2026-05-25"));
    }

    #[test]
    fn workday_saturday_to_friday() {
        // 2026-05-23 is a Saturday.
        assert_eq!(previous_workday(d("2026-05-23")), d("2026-05-22"));
    }

    #[test]
    fn workday_sunday_to_friday() {
        // 2026-05-24 is a Sunday.
        assert_eq!(previous_workday(d("2026-05-24")), d("2026-05-22"));
    }

    #[test]
    fn roundtrip_empty_store() {
        let dir = tempdir();
        let path = dir.join("entries.json");
        let store = Store::default();
        save(&path, &store).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, store);
    }

    #[test]
    fn roundtrip_with_entries() {
        let dir = tempdir();
        let path = dir.join("entries.json");
        let mut store = Store::default();
        store.entry_mut(d("2026-05-22")).did = vec!["a".into(), "b".into()];
        store.entry_mut(d("2026-05-22")).planning = vec!["c".into()];
        save(&path, &store).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded, store);
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = tempdir();
        let path = dir.join("nope.json");
        let loaded = load(&path).unwrap();
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn dates_desc_skips_empty_and_orders_newest_first() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-20")).did = vec!["x".into()];
        store.entry_mut(d("2026-05-21")); // empty, should be skipped
        store.entry_mut(d("2026-05-22")).planning = vec!["y".into()];
        assert_eq!(
            store.dates_desc(),
            vec![d("2026-05-22"), d("2026-05-20")],
        );
    }

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "standup-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
}
