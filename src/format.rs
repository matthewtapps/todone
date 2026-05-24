use chrono::NaiveDate;

use crate::storage::{Entry, Store, previous_workday};

/// A line starting with `- ` (after trimming) denotes a sub-item nested under
/// the most recent top-level item. Returns `(level, content)` where level is
/// 0 for top-level and 1 for sub-items.
pub fn parse_item(item: &str) -> (usize, &str) {
    if let Some(rest) = item.strip_prefix("- ") {
        (1, rest)
    } else {
        (0, item)
    }
}

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
    let mut i = 0;
    while i < items.len() {
        let (level, content) = parse_item(&items[i]);
        if level == 0 {
            out.push_str("<li>");
            push_html_escaped(out, content);
            // Lookahead: collect any sub-items immediately following this
            // parent into a nested <ul> *inside* its <li>.
            let mut j = i + 1;
            let mut sub_open = false;
            while j < items.len() {
                let (lvl, sub_content) = parse_item(&items[j]);
                if lvl == 0 {
                    break;
                }
                if !sub_open {
                    out.push_str("<ul>");
                    sub_open = true;
                }
                out.push_str("<li>");
                push_html_escaped(out, sub_content);
                out.push_str("</li>");
                j += 1;
            }
            if sub_open {
                out.push_str("</ul>");
            }
            out.push_str("</li>");
            i = j;
        } else {
            // Orphan sub-item (no preceding top-level): render as top-level
            // rather than dropping or wrapping in an empty <li>.
            out.push_str("<li>");
            push_html_escaped(out, content);
            out.push_str("</li>");
            i += 1;
        }
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
        let (level, content) = parse_item(item);
        out.push_str(match level {
            0 => "- ",
            _ => "  - ",
        });
        out.push_str(content);
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

    #[test]
    fn standup_html_nests_dash_prefixed_sub_items() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-22")).did = vec![
            "MR Review".into(),
            "- Session data polling".into(),
            "Navigation sidebar feature".into(),
        ];
        store.entry_mut(d("2026-05-25")).planning = vec!["x".into()];
        let out = standup_html(&store, d("2026-05-25"));
        assert_eq!(
            out,
            "<b>Yesterday</b><ul>\
             <li>MR Review<ul><li>Session data polling</li></ul></li>\
             <li>Navigation sidebar feature</li>\
             </ul><b>Today</b><ul><li>x</li></ul>"
        );
    }

    #[test]
    fn standup_html_groups_multiple_sub_items() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-22")).did = vec![
            "parent".into(),
            "- sub a".into(),
            "- sub b".into(),
            "next parent".into(),
        ];
        store.entry_mut(d("2026-05-25")).planning = vec!["x".into()];
        let out = standup_html(&store, d("2026-05-25"));
        assert!(out.contains(
            "<li>parent<ul><li>sub a</li><li>sub b</li></ul></li><li>next parent</li>"
        ));
    }

    #[test]
    fn standup_html_orphan_sub_item_becomes_top_level() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-22")).did = vec!["- orphan".into(), "parent".into()];
        store.entry_mut(d("2026-05-25")).planning = vec!["x".into()];
        let out = standup_html(&store, d("2026-05-25"));
        assert!(out.contains("<ul><li>orphan</li><li>parent</li></ul>"));
    }

    #[test]
    fn timesheet_keeps_dash_prefix_on_sub_items() {
        let mut store = Store::default();
        store.entry_mut(d("2026-05-22")).did = vec![
            "MR Review".into(),
            "- Session data polling".into(),
            "Navigation sidebar feature".into(),
        ];
        let out = timesheet(&store, d("2026-05-22"));
        assert_eq!(
            out,
            "MR Review\n- Session data polling\nNavigation sidebar feature"
        );
    }

    #[test]
    fn bullets_indents_sub_items() {
        let out = bullets(&[
            "parent".into(),
            "- sub".into(),
            "other".into(),
        ]);
        assert_eq!(out, "- parent\n  - sub\n- other");
    }
}
