use std::collections::HashMap;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone};
use chrono_tz::Tz;
use icalendar::{
    Calendar, CalendarComponent, CalendarDateTime, Component, DatePerhapsTime, Event, EventLike,
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

/// Raw VEVENT, retained alongside extracted view fields. The underlying
/// `Event` is kept so that recurrence expansion can call `get_recurrence()`
/// to assemble the RRuleSet from DTSTART/RRULE/RDATE/EXDATE.
#[derive(Debug, Clone)]
pub struct RawVevent {
    pub uid: String,
    pub summary: String,
    pub location: Option<String>,
    pub start: DatePerhapsTime,
    pub end: Option<DatePerhapsTime>,
    pub recurrence_id: Option<DatePerhapsTime>,
    pub status: Option<String>,
    pub has_rrule: bool,
    pub event: Event,
}

impl RawVevent {
    pub fn is_recurring(&self) -> bool {
        self.has_rrule
    }

    pub fn is_cancelled(&self) -> bool {
        self.status.as_deref() == Some("CANCELLED")
    }

    pub fn is_override(&self) -> bool {
        self.recurrence_id.is_some()
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
    Some(RawVevent {
        uid: ev.get_uid().unwrap_or("").to_string(),
        summary: ev.get_summary().unwrap_or("(no title)").to_string(),
        location: ev
            .property_value("LOCATION")
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        start,
        end,
        recurrence_id: ev.get_recurrence_id(),
        status: ev.property_value("STATUS").map(str::to_owned),
        has_rrule: ev.property_value("RRULE").is_some(),
        event: ev.clone(),
    })
}

/// Materialise concrete instances from `raws` covering `[window_start, window_end)`.
/// Recurring events are expanded via `rrule`; RECURRENCE-ID overrides replace or
/// suppress (when STATUS:CANCELLED) the matching expanded instance.
///
/// Bounding the expansion to a small window is essential — unbounded recurrences
/// (no UNTIL/COUNT) only terminate because of the window.
pub fn materialize(
    raws: &[RawVevent],
    window_start: DateTime<Local>,
    window_end: DateTime<Local>,
) -> Vec<CalendarEvent> {
    let mut out = Vec::new();

    // Split overrides from masters. Overrides are keyed by (uid, the original
    // instance's start) so the recurrence expander can swap or skip them.
    let masters: Vec<&RawVevent> = raws.iter().filter(|r| !r.is_override()).collect();
    let master_uids: std::collections::HashSet<&str> =
        masters.iter().map(|r| r.uid.as_str()).collect();

    let mut overrides: HashMap<(String, DateTime<Local>), &RawVevent> = HashMap::new();
    let mut orphan_overrides: Vec<&RawVevent> = Vec::new();
    for raw in raws.iter().filter(|r| r.is_override()) {
        let Some(rid) = raw.recurrence_id.as_ref() else {
            continue;
        };
        let Some(rid_local) = date_perhaps_time_to_local(rid) else {
            continue;
        };
        if master_uids.contains(raw.uid.as_str()) {
            overrides.insert((raw.uid.clone(), rid_local), raw);
        } else {
            orphan_overrides.push(raw);
        }
    }

    for master in &masters {
        if master.is_cancelled() {
            continue;
        }
        if master.is_recurring() {
            expand_recurring(master, &overrides, window_start, window_end, &mut out);
        } else if let Some(ev) = materialize_one(master) {
            push_if_in_window(ev, window_start, window_end, &mut out);
        }
    }

    // Overrides whose master isn't in the feed (rare). Treat as standalone.
    for orphan in orphan_overrides {
        if orphan.is_cancelled() {
            continue;
        }
        if let Some(ev) = materialize_one(orphan) {
            push_if_in_window(ev, window_start, window_end, &mut out);
        }
    }

    out
}

fn push_if_in_window(
    ev: CalendarEvent,
    window_start: DateTime<Local>,
    window_end: DateTime<Local>,
    out: &mut Vec<CalendarEvent>,
) {
    if ev.end > window_start && ev.start < window_end {
        out.push(ev);
    }
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

/// Expand a recurring VEVENT within `[window_start, window_end)`, substituting
/// or suppressing instances based on RECURRENCE-ID overrides.
fn expand_recurring(
    master: &RawVevent,
    overrides: &HashMap<(String, DateTime<Local>), &RawVevent>,
    window_start: DateTime<Local>,
    window_end: DateTime<Local>,
    out: &mut Vec<CalendarEvent>,
) {
    let Ok(set) = master.event.get_recurrence() else {
        return;
    };
    let duration = base_duration(master);
    let all_day = matches!(master.start, DatePerhapsTime::Date(_));

    let after = local_to_rrule_utc(window_start);
    let before = local_to_rrule_utc(window_end);
    // 1000 is comfortably above any realistic count over a few days even for
    // sub-hour recurrences. rrule won't iterate beyond `before` anyway.
    let result = set.after(after).before(before).all(1000);

    for occurrence in result.dates {
        let start = occurrence.with_timezone(&Local);
        let key = (master.uid.clone(), start);
        if let Some(override_raw) = overrides.get(&key) {
            if override_raw.is_cancelled() {
                continue;
            }
            if let Some(ev) = materialize_one(override_raw) {
                push_if_in_window(ev, window_start, window_end, out);
            }
        } else {
            let ev = CalendarEvent {
                uid: master.uid.clone(),
                start,
                end: start + duration,
                all_day,
                summary: master.summary.clone(),
                location: master.location.clone(),
            };
            push_if_in_window(ev, window_start, window_end, out);
        }
    }
}

fn base_duration(raw: &RawVevent) -> Duration {
    resolve_times(&raw.start, raw.end.as_ref())
        .map(|(s, e, _)| e - s)
        .unwrap_or_else(Duration::zero)
}

fn date_perhaps_time_to_local(d: &DatePerhapsTime) -> Option<DateTime<Local>> {
    match d {
        DatePerhapsTime::Date(d) => local_midnight(*d),
        DatePerhapsTime::DateTime(cdt) => cdt_to_local(cdt),
    }
}

fn local_to_rrule_utc(dt: DateTime<Local>) -> DateTime<rrule::Tz> {
    let utc = dt.with_timezone(&chrono::Utc);
    rrule::Tz::UTC.from_utc_datetime(&utc.naive_utc())
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

/// Convenience for the renderer: materialise just the instances that fall on
/// `date` (local time), sorted by start. Recurrence expansion is bounded to
/// that single day so it stays cheap.
pub fn events_for_date(raws: &[RawVevent], date: NaiveDate) -> Vec<CalendarEvent> {
    let Some(start) = local_midnight(date) else {
        return Vec::new();
    };
    let Some(end) = date.succ_opt().and_then(local_midnight) else {
        return Vec::new();
    };
    let mut events = materialize(raws, start, end);
    events.sort_by_key(|e| (e.start, e.end));
    events
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

    /// Window large enough to include every event in the fixture (single,
    /// all-day, multi-day, and ~5 weeks of the weekly recurrence).
    fn wide_window() -> (DateTime<Local>, DateTime<Local>) {
        let start = local_midnight(NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()).unwrap();
        let end = local_midnight(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()).unwrap();
        (start, end)
    }

    #[test]
    fn parses_single_timed_event() {
        let raws = parse_ics(FIXTURE).unwrap();
        let single = raws
            .iter()
            .find(|r| r.uid == "single@example.com")
            .unwrap();
        assert_eq!(single.summary, "Single Standup");
        assert!(!single.is_recurring());
    }

    #[test]
    fn materialises_all_day_event_with_day_long_span() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
        let all_day = events
            .iter()
            .find(|e| e.uid == "allday@example.com")
            .unwrap();
        assert!(all_day.all_day);
        assert_eq!(all_day.end - all_day.start, Duration::days(1));
    }

    #[test]
    fn materialises_multi_day_event() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
        let multi = events
            .iter()
            .find(|e| e.uid == "multiday@example.com")
            .unwrap();
        assert!(multi.all_day);
        assert_eq!(multi.end - multi.start, Duration::days(3));
    }

    #[test]
    fn drops_cancelled_master_events() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
        assert!(events.iter().all(|e| e.uid != "cancelled@example.com"));
    }

    #[test]
    fn events_on_picks_overlapping_only() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
        let on_day = events_on(&events, NaiveDate::from_ymd_opt(2026, 5, 22).unwrap());
        assert!(on_day.iter().any(|e| e.uid == "single@example.com"));
        let off_day = events_on(&events, NaiveDate::from_ymd_opt(2026, 5, 21).unwrap());
        assert!(off_day.iter().all(|e| e.uid != "single@example.com"));
    }

    #[test]
    fn events_on_includes_multi_day_for_each_covered_day() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
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
        let after = events_on(&events, NaiveDate::from_ymd_opt(2026, 6, 4).unwrap());
        assert!(after.iter().all(|e| e.uid != "multiday@example.com"));
    }

    #[test]
    fn expands_weekly_recurrence_within_window() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
        // 2026-05-25 (the DTSTART itself) should appear.
        let on = events_on(&events, NaiveDate::from_ymd_opt(2026, 5, 25).unwrap());
        assert!(
            on.iter().any(|e| e.uid == "weekly@example.com" && !e.summary.contains("rescheduled")),
            "first weekly instance should appear"
        );
        // 2026-06-22 is 4 weeks later — still expanded.
        let later = events_on(&events, NaiveDate::from_ymd_opt(2026, 6, 22).unwrap());
        assert!(
            later.iter().any(|e| e.uid == "weekly@example.com"),
            "later weekly instance should appear"
        );
    }

    #[test]
    fn exdate_suppresses_instance() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
        let on = events_on(&events, NaiveDate::from_ymd_opt(2026, 6, 1).unwrap());
        assert!(
            on.iter().all(|e| e.uid != "weekly@example.com"),
            "EXDATE'd weekly instance must not appear"
        );
    }

    #[test]
    fn recurrence_id_override_replaces_instance() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
        // Original 2026-06-08 should be replaced by override on 2026-06-09 11:00.
        let on_original = events_on(&events, NaiveDate::from_ymd_opt(2026, 6, 8).unwrap());
        assert!(
            on_original.iter().all(|e| e.uid != "weekly@example.com"),
            "overridden instance must not appear on its original date"
        );
        let on_new = events_on(&events, NaiveDate::from_ymd_opt(2026, 6, 9).unwrap());
        let moved = on_new
            .iter()
            .find(|e| e.uid == "weekly@example.com")
            .expect("override should appear on its new date");
        assert!(moved.summary.contains("rescheduled"));
        assert_eq!(moved.start.format("%H:%M").to_string(), "11:00");
    }

    #[test]
    fn cancelled_override_suppresses_instance() {
        let raws = parse_ics(FIXTURE).unwrap();
        let (s, e) = wide_window();
        let events = materialize(&raws, s, e);
        let on = events_on(&events, NaiveDate::from_ymd_opt(2026, 6, 15).unwrap());
        assert!(
            on.iter().all(|e| e.uid != "weekly@example.com"),
            "cancelled override must suppress the instance"
        );
    }
}
