use chrono::NaiveDate;

use crate::storage::{Entry, Store, previous_workday};

/// Teams-paste standup for a given day, as HTML to be put on the clipboard
/// with the `text/html` MIME type. Teams renders this as bold headings
/// followed by bullet lists.
///
/// Shape: `<b>Yesterday</b><ul><li>...</li>...</ul><b>Today</b><ul>...</ul>`.
/// Empty sections substitute a `<br>` for the `<ul>` so the next heading
/// doesn't end up inline with the previous one.
pub fn standup_html(store: &Store, day: NaiveDate) -> String {
    let yest = previous_workday(day);
    let did = store.get(yest).map(|e| &e.did[..]).unwrap_or(&[]);
    let planning = store.get(day).map(|e| &e.planning[..]).unwrap_or(&[]);

    let mut out = String::new();
    out.push_str("<b>Yesterday</b>");
    push_html_section(&mut out, did);
    out.push_str("<b>Today</b>");
    push_html_section(&mut out, planning);
    out
}

fn push_html_section(out: &mut String, items: &[String]) {
    if items.is_empty() {
        out.push_str("<br>");
        return;
    }
    out.push_str("<ul>");
    for item in items {
        out.push_str("<li>");
        push_html_escaped(out, item);
        out.push_str("</li>");
    }
    out.push_str("</ul>");
}

fn push_html_escaped(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
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
    fn standup_html_pulls_yesterday_from_previous_workday() {
        let mut store = Store::default();
        // Friday: what I did
        store.entry_mut(d("2026-05-22")).did =
            vec!["finished feature X".into(), "reviewed PRs".into()];
        // Monday: my plan
        store.entry_mut(d("2026-05-25")).planning = vec!["start feature Y".into()];

        let out = standup_html(&store, d("2026-05-25"));
        assert_eq!(
            out,
            "<b>Yesterday</b><ul><li>finished feature X</li><li>reviewed PRs</li></ul>\
             <b>Today</b><ul><li>start feature Y</li></ul>"
        );
    }

    #[test]
    fn standup_html_uses_br_when_a_section_is_empty() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-26")).planning = vec!["thing".into()];
        let out = standup_html(&store, d("2026-05-26"));
        assert_eq!(out, "<b>Yesterday</b><br><b>Today</b><ul><li>thing</li></ul>");
    }

    #[test]
    fn standup_html_escapes_special_chars() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-22")).did = vec!["a & b <c> \"d\"".into()];
        store.entry_mut(d("2026-05-25")).planning = vec!["x".into()];
        let out = standup_html(&store, d("2026-05-25"));
        assert!(out.contains("<li>a &amp; b &lt;c&gt; \"d\"</li>"));
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
