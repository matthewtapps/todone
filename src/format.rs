use chrono::NaiveDate;

use crate::storage::{Entry, Store, previous_workday};

/// Teams-style standup for a given day:
///   **Yesterday** = previous_workday(day).did
///   **Today**     = day.planning
pub fn standup(store: &Store, day: NaiveDate) -> String {
    let yest = previous_workday(day);
    let did = store.get(yest).map(|e| &e.did[..]).unwrap_or(&[]);
    let planning = store.get(day).map(|e| &e.planning[..]).unwrap_or(&[]);

    let mut out = String::new();
    out.push_str("**Yesterday**\n");
    push_bullets(&mut out, did);
    out.push('\n');
    out.push_str("**Today**\n");
    push_bullets(&mut out, planning);
    out
}

/// Timesheet for a given day: the `did` bullets, plain text, one per line, no prefix.
pub fn timesheet(store: &Store, day: NaiveDate) -> String {
    let did = store.get(day).map(|e| &e.did[..]).unwrap_or(&[]);
    did.iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Just one section's bullets as `- ` prefixed lines, for yanking a single section.
pub fn bullets(items: &[String]) -> String {
    let mut out = String::new();
    push_bullets(&mut out, items);
    // trim trailing newline, since yank-just-this-section shouldn't have one
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

fn push_bullets(out: &mut String, items: &[String]) {
    for item in items {
        out.push_str("- ");
        out.push_str(item);
        out.push('\n');
    }
}

pub fn did(entry: &Entry) -> String {
    bullets(&entry.did)
}

pub fn planning(entry: &Entry) -> String {
    bullets(&entry.planning)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn standup_pulls_yesterday_from_previous_workday() {
        let mut store = Store::default();
        // Friday: what I did
        store.entry_mut(d("2026-05-22")).did = vec!["finished feature X".into(), "reviewed PRs".into()];
        // Monday: my plan
        store.entry_mut(d("2026-05-25")).planning = vec!["start feature Y".into()];

        let out = standup(&store, d("2026-05-25"));
        assert_eq!(
            out,
            "**Yesterday**\n- finished feature X\n- reviewed PRs\n\n**Today**\n- start feature Y\n"
        );
    }

    #[test]
    fn standup_handles_missing_yesterday() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-26")).planning = vec!["thing".into()];
        let out = standup(&store, d("2026-05-26"));
        assert_eq!(out, "**Yesterday**\n\n**Today**\n- thing\n");
    }

    #[test]
    fn timesheet_plain_no_prefix() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-22")).did = vec!["a".into(), "b".into(), "c".into()];
        let out = timesheet(&store, d("2026-05-22"));
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn timesheet_empty_when_no_entry() {
        let store = Store::default();
        let out = timesheet(&store, d("2026-05-22"));
        assert_eq!(out, "");
    }

    #[test]
    fn bullets_trims_trailing_newline() {
        let out = bullets(&["x".into(), "y".into()]);
        assert_eq!(out, "- x\n- y");
    }
}
