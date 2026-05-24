use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone};
use chrono_tz::Tz;
use icalendar::{
    Calendar, CalendarComponent, CalendarDateTime, Component, DatePerhapsTime, Event,
};

/// A concrete calendar event instance, ready to render. For all-day events
/// `start`/`end` are the local-time midnight boundaries (`end` exclusive),
/// matching how iCalendar's DATE values behave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEvent {
    pub uid: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub all_day: bool,
    pub summary: String,
    pub location: Option<String>,
}

/// Raw VEVENT data, keeping recurrence properties intact so a later phase can
/// expand them. Phase 2 only materialises VEVENTs that have no RRULE.
#[derive(Debug, Clone)]
pub struct RawVevent {
    pub uid: String,
    pub summary: String,
    pub location: Option<String>,
    pub start: DatePerhapsTime,
    pub end: Option<DatePerhapsTime>,
    pub rrule: Option<String>,
    pub exdates: Vec<String>,
    pub rdates: Vec<String>,
    pub recurrence_id: Option<DatePerhapsTime>,
    pub status: Option<String>,
}

impl RawVevent {
    pub fn is_recurring(&self) -> bool {
        self.rrule.is_some()
    }

    pub fn is_cancelled(&self) -> bool {
        self.status.as_deref() == Some("CANCELLED")
    }
}

/// Parse an ICS document into raw VEVENTs. Components that aren't VEVENTs
/// (VTIMEZONE, VCALENDAR-level props, etc.) are ignored. Malformed VEVENTs
/// missing DTSTART are skipped silently — one bad event shouldn't poison the
/// whole feed.
pub fn parse_ics(text: &str) -> Result<Vec<RawVevent>> {
    let cal = Calendar::from_str(text).map_err(|e| anyhow!("parsing ICS: {e}"))?;
    let mut out = Vec::new();
    for comp in &cal.components {
        let CalendarComponent::Event(ev) = comp else { continue };
        if let Some(raw) = raw_from_event(ev) {
            out.push(raw);
        }
    }
    Ok(out)
}

fn raw_from_event(ev: &Event) -> Option<RawVevent> {
    let start = ev.get_start()?;
    let end = ev.get_end();
    let exdates = ev
        .multi_properties()
        .get("EXDATE")
        .into_iter()
        .flatten()
        .map(|p| p.value().to_string())
        .collect();
    let rdates = ev
        .properties()
        .get("RDATE")
        .map(|p| vec![p.value().to_string()])
        .unwrap_or_default();
    Some(RawVevent {
        uid: ev.get_uid().unwrap_or("").to_string(),
        summary: ev.get_summary().unwrap_or("(no title)").to_string(),
        location: ev
            .property_value("LOCATION")
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        start,
        end,
        rrule: ev.property_value("RRULE").map(str::to_owned),
        exdates,
        rdates,
        recurrence_id: ev.get_recurrence_id(),
        status: ev.property_value("STATUS").map(str::to_owned),
    })
}

/// Materialise all non-recurring events from `raws`. Cancelled VEVENTs are
/// dropped. Recurring events are deferred to a later phase.
pub fn materialize_non_recurring(raws: &[RawVevent]) -> Vec<CalendarEvent> {
    raws.iter()
        .filter(|r| !r.is_recurring() && !r.is_cancelled())
        .filter_map(materialize_one)
        .collect()
}

fn materialize_one(raw: &RawVevent) -> Option<CalendarEvent> {
    let (start, end, all_day) = resolve_times(&raw.start, raw.end.as_ref())?;
    Some(CalendarEvent {
        uid: raw.uid.clone(),
        start,
        end,
        all_day,
        summary: raw.summary.clone(),
        location: raw.location.clone(),
    })
}

/// Convert a VEVENT's DTSTART/DTEND into local-time bounds. For all-day
/// events we synthesize a 1-day window when DTEND is missing (per RFC 5545
/// §3.6.1: a VEVENT with DTSTART as DATE and no DTEND lasts one day).
fn resolve_times(
    start: &DatePerhapsTime,
    end: Option<&DatePerhapsTime>,
) -> Option<(DateTime<Local>, DateTime<Local>, bool)> {
    match start {
        DatePerhapsTime::Date(d) => {
            let s = local_midnight(*d)?;
            let end_date = match end {
                Some(DatePerhapsTime::Date(ed)) => *ed,
                _ => d.succ_opt()?,
            };
            let e = local_midnight(end_date)?;
            Some((s, e, true))
        }
        DatePerhapsTime::DateTime(cdt) => {
            let s = cdt_to_local(cdt)?;
            let e = match end {
                Some(DatePerhapsTime::DateTime(ecdt)) => cdt_to_local(ecdt)?,
                Some(DatePerhapsTime::Date(d)) => local_midnight(*d)?,
                None => s,
            };
            Some((s, e, false))
        }
    }
}

fn local_midnight(d: NaiveDate) -> Option<DateTime<Local>> {
    d.and_time(NaiveTime::MIN)
        .and_local_timezone(Local)
        .single()
}

/// Convert a CalendarDateTime to local time. `Floating` values have no zone —
/// we treat them as already-local, matching the "attendee's current zone"
/// behavior in RFC 5545 §3.3.5.
fn cdt_to_local(cdt: &CalendarDateTime) -> Option<DateTime<Local>> {
    match cdt {
        CalendarDateTime::Utc(dt) => Some(dt.with_timezone(&Local)),
        CalendarDateTime::Floating(ndt) => ndt.and_local_timezone(Local).single(),
        CalendarDateTime::WithTimezone { date_time, tzid } => {
            let tz: Tz = tzid.parse().ok()?;
            tz.from_local_datetime(date_time)
                .single()
                .map(|d| d.with_timezone(&Local))
        }
    }
}

/// Filter `events` to those whose `[start, end)` interval overlaps `date` in
/// local time. Multi-day events show up on every day they cover.
pub fn events_on<'a>(events: &'a [CalendarEvent], date: NaiveDate) -> Vec<&'a CalendarEvent> {
    let Some(day_start) = local_midnight(date) else {
        return Vec::new();
    };
    let Some(next) = date.succ_opt() else {
        return Vec::new();
    };
    let Some(day_end) = local_midnight(next) else {
        return Vec::new();
    };
    let mut out: Vec<&CalendarEvent> = events
        .iter()
        .filter(|e| e.start < day_end && e.end > day_start)
        .collect();
    out.sort_by_key(|e| (e.start, e.end));
    out
}

#[derive(Clone)]
pub struct Client {
    url: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(url: &str) -> Result<Self> {
        let url = url.trim().to_string();
        if url.is_empty() {
            return Err(anyhow!("ICS URL is empty"));
        }
        let http = reqwest::Client::builder()
            .user_agent("standup")
            .build()
            .context("building HTTP client")?;
        Ok(Self { url, http })
    }

    /// Fetch and parse the calendar. Returns raw VEVENTs; call
    /// `materialize_non_recurring` (Phase 2) or the upcoming recurrence
    /// expander (Phase 3) to get concrete events.
    pub async fn fetch(&self) -> Result<Vec<RawVevent>> {
        let body = self
            .http
            .get(&self.url)
            .send()
            .await
            .with_context(|| format!("GET {}", self.url))?
            .error_for_status()
            .context("ICS endpoint returned an error status")?
            .text()
            .await
            .context("reading ICS body")?;
        parse_ics(&body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/calendar_basic.ics");

    #[test]
    fn parses_single_timed_event() {
        let raws = parse_ics(FIXTURE).unwrap();
        let single = raws.iter().find(|r| r.uid == "single@example.com").unwrap();
        assert_eq!(single.summary, "Single Standup");
        assert!(!single.is_recurring());
    }

    #[test]
    fn parses_all_day_event() {
        let raws = parse_ics(FIXTURE).unwrap();
        let all_day = raws.iter().find(|r| r.uid == "allday@example.com").unwrap();
        let events = materialize_non_recurring(&[all_day.clone()]);
        assert_eq!(events.len(), 1);
        assert!(events[0].all_day);
        // All-day end is the next day's midnight (RFC 5545 exclusive).
        assert_eq!(
            (events[0].end - events[0].start),
            chrono::Duration::days(1)
        );
    }

    #[test]
    fn parses_multi_day_event() {
        let raws = parse_ics(FIXTURE).unwrap();
        let multi = raws.iter().find(|r| r.uid == "multiday@example.com").unwrap();
        let events = materialize_non_recurring(&[multi.clone()]);
        assert_eq!(events.len(), 1);
        assert!(events[0].all_day);
        assert_eq!(events[0].end - events[0].start, chrono::Duration::days(3));
    }

    #[test]
    fn skips_recurring_events_in_phase_2() {
        let raws = parse_ics(FIXTURE).unwrap();
        let has_recurring = raws.iter().any(|r| r.is_recurring());
        assert!(has_recurring, "fixture should contain a recurring event");
        let events = materialize_non_recurring(&raws);
        assert!(events.iter().all(|e| e.uid != "weekly@example.com"));
    }

    #[test]
    fn events_on_picks_overlapping_only() {
        let raws = parse_ics(FIXTURE).unwrap();
        let events = materialize_non_recurring(&raws);
        // Fixture's single event is on 2026-05-22.
        let on_day = events_on(&events, NaiveDate::from_ymd_opt(2026, 5, 22).unwrap());
        assert!(on_day.iter().any(|e| e.uid == "single@example.com"));
        let off_day = events_on(&events, NaiveDate::from_ymd_opt(2026, 5, 21).unwrap());
        assert!(off_day.iter().all(|e| e.uid != "single@example.com"));
    }

    #[test]
    fn events_on_includes_multi_day_for_each_covered_day() {
        let raws = parse_ics(FIXTURE).unwrap();
        let events = materialize_non_recurring(&raws);
        // Multi-day runs 2026-06-01..2026-06-04.
        for day in [
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 2).unwrap(),
            NaiveDate::from_ymd_opt(2026, 6, 3).unwrap(),
        ] {
            let on = events_on(&events, day);
            assert!(
                on.iter().any(|e| e.uid == "multiday@example.com"),
                "multi-day should appear on {day}"
            );
        }
        // Excludes the exclusive end date.
        let after = events_on(&events, NaiveDate::from_ymd_opt(2026, 6, 4).unwrap());
        assert!(after.iter().all(|e| e.uid != "multiday@example.com"));
    }

    #[test]
    fn drops_cancelled_events() {
        let raws = parse_ics(FIXTURE).unwrap();
        let events = materialize_non_recurring(&raws);
        assert!(events.iter().all(|e| e.uid != "cancelled@example.com"));
    }
}
