use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate};
use serde::Serialize;

use crate::{calendar, config, storage};

#[derive(Serialize, Default)]
pub struct Day {
    did: Vec<String>,
    events: Vec<Event>,
}

#[derive(Serialize)]
struct Event {
    summary: String,
    start: String,
    end: String,
    all_day: bool,
}

/// Fetch and parse the configured calendar feed, or `None` when it's disabled.
pub async fn fetch_calendar() -> Result<Option<Vec<calendar::RawVevent>>> {
    let settings = config::load(&config::default_path()?)?;
    if !settings.calendar.enabled || settings.calendar.ics_url.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(calendar::Client::new(&settings.calendar.ics_url)?.fetch().await?))
}

/// Entries and calendar events per day, inclusive of both dates.
pub fn days(
    from: NaiveDate,
    to: NaiveDate,
    raws: Option<&[calendar::RawVevent]>,
) -> Result<BTreeMap<NaiveDate, Day>> {
    let store = storage::load(&storage::default_path()?)?;
    let mut days = BTreeMap::new();
    let mut d = from;
    while d <= to {
        let did = store.get(d).map(|e| e.did.clone()).unwrap_or_default();
        let events = raws
            .map(|r| calendar::events_for_date(r, d))
            .unwrap_or_default()
            .into_iter()
            .map(|ev| Event {
                summary: ev.summary,
                start: ev.start.to_rfc3339(),
                end: ev.end.to_rfc3339(),
                all_day: ev.all_day,
            })
            .collect();
        days.insert(d, Day { did, events });
        d += Duration::days(1);
    }
    Ok(days)
}

/// `todone export FROM TO` prints entries and calendar events per day as JSON.
pub fn run(args: &[String], runtime: &tokio::runtime::Runtime) -> Result<()> {
    let [from, to] = args else {
        anyhow::bail!("usage: todone export YYYY-MM-DD YYYY-MM-DD");
    };
    let from: NaiveDate = from.parse().context("parsing FROM")?;
    let to: NaiveDate = to.parse().context("parsing TO")?;
    let raws = runtime.block_on(fetch_calendar())?;
    println!("{}", serde_json::to_string_pretty(&days(from, to, raws.as_deref())?)?);
    Ok(())
}
