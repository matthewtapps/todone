use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;

/// One event from `/api/v4/events` kept as raw JSON — easier to evolve the
/// summariser against real shapes than to fight a typed schema.
pub type RawEvent = Value;

/// Nerd-font glyphs used in the GitLab context tab.
pub const ICON_PUSH: &str = "\u{f126}"; //
pub const ICON_COMMENT: &str = "\u{f075}"; //
pub const ICON_APPROVE: &str = "\u{f00c}"; //
pub const ICON_OPEN: &str = "\u{f067}"; //
pub const ICON_MERGE: &str = "\u{f407}"; //
pub const ICON_CLOSE: &str = "\u{f00d}"; //
pub const ICON_OTHER: &str = "\u{f129}"; //

#[derive(Debug, Clone)]
pub struct SummaryLine {
    pub icon: &'static str,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ProjectGroup {
    pub project: String,
    pub lines: Vec<SummaryLine>,
}

#[derive(Default)]
struct Bucket {
    /// branch -> (push_count, commit_total)
    pushes: BTreeMap<String, (usize, u64)>,
    /// (kind, title, mr_author) -> count, in first-seen order. `mr_author` is
    /// only populated for comments on merge requests; empty otherwise.
    comments: Vec<((String, String, String), usize)>,
    other: Vec<SummaryLine>,
}

/// Group a day of raw events under their project. Within each group:
/// pushes (collapsed by branch) → comments (collapsed by target) → other
/// actions in input order.
pub fn summarise(events: &[RawEvent]) -> Vec<ProjectGroup> {
    let mut per_project: BTreeMap<String, Bucket> = BTreeMap::new();

    for ev in events {
        let action = ev.get("action_name").and_then(Value::as_str).unwrap_or("");
        let target_type = ev.get("target_type").and_then(Value::as_str).unwrap_or("");
        let target_title = ev
            .get("target_title")
            .and_then(Value::as_str)
            .unwrap_or("");
        let project = event_project(ev);
        let bucket = per_project.entry(project).or_default();

        match action {
            "pushed to" | "pushed new" | "pushed" => {
                let branch = ev
                    .pointer("/push_data/ref")
                    .and_then(Value::as_str)
                    .unwrap_or("(unknown)");
                let commits = ev
                    .pointer("/push_data/commit_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let entry = bucket.pushes.entry(branch.to_string()).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += commits;
            }
            "opened" => bucket.other.push(SummaryLine {
                icon: ICON_OPEN,
                text: format!(
                    "Opened {}: {target_title}{}",
                    pretty_target(target_type),
                    mr_author_suffix(ev)
                ),
            }),
            "closed" => bucket.other.push(SummaryLine {
                icon: ICON_CLOSE,
                text: format!(
                    "Closed {}: {target_title}{}",
                    pretty_target(target_type),
                    mr_author_suffix(ev)
                ),
            }),
            "accepted" | "merged" => bucket.other.push(SummaryLine {
                icon: ICON_MERGE,
                text: format!("Merged MR: {target_title}{}", mr_author_suffix(ev)),
            }),
            "approved" => bucket.other.push(SummaryLine {
                icon: ICON_APPROVE,
                text: format!("Approved: {target_title}{}", mr_author_suffix(ev)),
            }),
            "commented on" => {
                let noteable_type = ev
                    .pointer("/note/noteable_type")
                    .and_then(Value::as_str)
                    .unwrap_or(target_type);
                let author = event_mr_author(ev).unwrap_or("").to_string();
                let key = (
                    pretty_target(noteable_type).to_string(),
                    target_title.to_string(),
                    author,
                );
                if let Some(slot) = bucket.comments.iter_mut().find(|(k, _)| *k == key) {
                    slot.1 += 1;
                } else {
                    bucket.comments.push((key, 1));
                }
            }
            other_action => {
                let text = if target_title.is_empty() {
                    other_action.to_string()
                } else {
                    format!("{other_action}: {target_title}")
                };
                bucket.other.push(SummaryLine { icon: ICON_OTHER, text });
            }
        }
    }

    let mut groups = Vec::with_capacity(per_project.len());
    for (project, bucket) in per_project {
        let Bucket { pushes, comments, other } = bucket;
        let mut lines = Vec::with_capacity(pushes.len() + comments.len() + other.len());
        for (branch, (push_count, commit_count)) in pushes {
            let suffix = match (push_count, commit_count) {
                (1, 0) => String::new(),
                (1, c) => format!(" ({c} commit{})", plural(c)),
                (p, 0) => format!(" ({p} push{})", plural_es(p as u64)),
                (p, c) => format!(
                    " ({p} push{}, {c} commit{})",
                    plural_es(p as u64),
                    plural(c)
                ),
            };
            lines.push(SummaryLine {
                icon: ICON_PUSH,
                text: format!("{branch}{suffix}"),
            });
        }
        for ((kind, title, author), count) in comments {
            let count_suffix = if count > 1 {
                format!(" ({count} comments)")
            } else {
                String::new()
            };
            let author_suffix = if author.is_empty() {
                String::new()
            } else {
                format!(" (by {author})")
            };
            let text = if title.is_empty() {
                format!("Commented on {kind}{author_suffix}{count_suffix}")
            } else {
                format!("Commented on {kind}: {title}{author_suffix}{count_suffix}")
            };
            lines.push(SummaryLine { icon: ICON_COMMENT, text });
        }
        lines.extend(other);
        groups.push(ProjectGroup { project, lines });
    }
    groups
}

/// Returns `(project_id, mr_iid)` if the event references a merge request —
/// either directly via `target_type=MergeRequest` or indirectly via a note on
/// one. Used to fan out author lookups during enrichment.
fn event_mr_ref(ev: &RawEvent) -> Option<(u64, u64)> {
    let project_id = ev.get("project_id").and_then(Value::as_u64)?;
    let target_type = ev.get("target_type").and_then(Value::as_str).unwrap_or("");
    if target_type == "MergeRequest" {
        if let Some(iid) = ev.get("target_iid").and_then(Value::as_u64) {
            return Some((project_id, iid));
        }
    }
    let noteable_type = ev
        .pointer("/note/noteable_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    if noteable_type == "MergeRequest" {
        if let Some(iid) = ev.pointer("/note/noteable_iid").and_then(Value::as_u64) {
            return Some((project_id, iid));
        }
    }
    None
}

fn event_mr_author(ev: &RawEvent) -> Option<&str> {
    ev.get("_mr_author_username").and_then(Value::as_str)
}

fn mr_author_suffix(ev: &RawEvent) -> String {
    match event_mr_author(ev) {
        Some(u) => format!(" (by {u})"),
        None => String::new(),
    }
}

fn event_project(ev: &RawEvent) -> String {
    if let Some(name) = ev.get("_project_name").and_then(Value::as_str) {
        return short_project(name).to_string();
    }
    if let Some(id) = ev.get("project_id").and_then(Value::as_u64) {
        return format!("#{id}");
    }
    "?".to_string()
}

fn short_project(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, p)| p).unwrap_or(path)
}

fn pretty_target(t: &str) -> &'static str {
    match t {
        "MergeRequest" => "MR",
        "Issue" => "issue",
        "DiffNote" | "Note" | "DiscussionNote" => "note",
        "Milestone" => "milestone",
        "WikiPage::Meta" | "WikiPage" => "wiki",
        "Commit" => "commit",
        _ => "item",
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn plural_es(n: u64) -> &'static str {
    if n == 1 { "" } else { "es" }
}

#[derive(Debug, Clone, Deserialize)]
struct User {
    id: u64,
}

#[derive(Clone)]
pub struct Client {
    base_url: String,
    client: reqwest::Client,
    /// project_id → `path_with_namespace`. Populated lazily on first sighting;
    /// shared across clones so resolving a project name once benefits later
    /// fetches.
    project_names: Arc<RwLock<HashMap<u64, String>>>,
    /// (project_id, mr_iid) → MR author username. Populated lazily; lookups
    /// missing from cache fall back to silently omitting the author rather
    /// than blocking the summary.
    mr_authors: Arc<RwLock<HashMap<(u64, u64), String>>>,
}

impl Client {
    pub fn new(instance_url: &str, token: &str) -> Result<Self> {
        let base_url = normalize_base_url(instance_url);
        let mut headers = HeaderMap::new();
        headers.insert(
            "PRIVATE-TOKEN",
            token
                .parse()
                .context("token contains invalid header bytes")?,
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent("todone")
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            base_url,
            client,
            project_names: Arc::new(RwLock::new(HashMap::new())),
            mr_authors: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn resolve_user(&self, username: &str) -> Result<u64> {
        let url = format!("{}/api/v4/users?username={}", self.base_url, username);
        let users: Vec<User> = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("looking up user {username}"))?
            .json()
            .await
            .context("parsing user lookup response")?;
        users
            .into_iter()
            .next()
            .map(|u| u.id)
            .ok_or_else(|| anyhow!("no GitLab user matches '{username}'"))
    }

    /// Fetch all events for `user_id` on `date` (paginated). Each returned
    /// event has its project_id resolved to a name and stored under the
    /// custom `_project_name` field so the summariser can use it.
    pub async fn fetch_events(&self, user_id: u64, date: NaiveDate) -> Result<Vec<RawEvent>> {
        // GitLab's `after` and `before` are exclusive day-precision filters.
        let after = date.pred_opt().unwrap_or(date);
        let before = date.succ_opt().unwrap_or(date);
        let mut all = Vec::new();
        let mut page = 1;
        loop {
            let url = format!(
                "{}/api/v4/users/{}/events?after={}&before={}&per_page=100&page={}",
                self.base_url, user_id, after, before, page
            );
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .with_context(|| format!("GET {url}"))?
                .error_for_status()
                .with_context(|| format!("fetching events for {date}"))?;
            let batch: Vec<RawEvent> = resp.json().await.context("parsing events response")?;
            if batch.is_empty() {
                break;
            }
            let got = batch.len();
            all.extend(batch);
            if got < 100 {
                break;
            }
            page += 1;
            if page > 20 {
                // Safety guard — 2k events on a single day is implausible.
                break;
            }
        }
        self.enrich_with_project_names(&mut all).await;
        self.enrich_with_mr_authors(&mut all).await;
        Ok(all)
    }

    async fn enrich_with_project_names(&self, events: &mut [RawEvent]) {
        let mut needed: HashSet<u64> = HashSet::new();
        {
            let cache = self.project_names.read().await;
            for ev in events.iter() {
                if let Some(id) = ev.get("project_id").and_then(Value::as_u64) {
                    if !cache.contains_key(&id) {
                        needed.insert(id);
                    }
                }
            }
        }
        // Resolve sequentially; a single day rarely touches more than a
        // handful of projects, and the cache makes this a one-off cost.
        for id in needed {
            if let Ok(name) = self.fetch_project_path(id).await {
                self.project_names.write().await.insert(id, name);
            }
        }
        let cache = self.project_names.read().await;
        for ev in events.iter_mut() {
            let Some(id) = ev.get("project_id").and_then(Value::as_u64) else {
                continue;
            };
            if let (Some(name), Value::Object(map)) = (cache.get(&id), ev) {
                map.insert("_project_name".into(), Value::String(name.clone()));
            }
        }
    }

    async fn enrich_with_mr_authors(&self, events: &mut [RawEvent]) {
        let mut needed: HashSet<(u64, u64)> = HashSet::new();
        {
            let cache = self.mr_authors.read().await;
            for ev in events.iter() {
                if let Some(key) = event_mr_ref(ev) {
                    if !cache.contains_key(&key) {
                        needed.insert(key);
                    }
                }
            }
        }
        for (project_id, iid) in needed {
            if let Ok(author) = self.fetch_mr_author(project_id, iid).await {
                self.mr_authors
                    .write()
                    .await
                    .insert((project_id, iid), author);
            }
        }
        let cache = self.mr_authors.read().await;
        for ev in events.iter_mut() {
            let Some(key) = event_mr_ref(ev) else { continue };
            if let (Some(author), Value::Object(map)) = (cache.get(&key), ev) {
                map.insert(
                    "_mr_author_username".into(),
                    Value::String(author.clone()),
                );
            }
        }
    }

    async fn fetch_mr_author(&self, project_id: u64, iid: u64) -> Result<String> {
        let url = format!(
            "{}/api/v4/projects/{}/merge_requests/{}",
            self.base_url, project_id, iid
        );
        let resp: Value = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("fetching MR {project_id}!{iid}"))?
            .json()
            .await
            .context("parsing MR response")?;
        resp.pointer("/author/username")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("MR {project_id}!{iid} response missing author username"))
    }

    async fn fetch_project_path(&self, project_id: u64) -> Result<String> {
        let url = format!("{}/api/v4/projects/{}", self.base_url, project_id);
        let resp: Value = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("fetching project {project_id}"))?
            .json()
            .await
            .context("parsing project response")?;
        resp.get("path_with_namespace")
            .or_else(|| resp.get("path"))
            .or_else(|| resp.get("name"))
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| anyhow!("project {project_id} response missing name"))
    }
}

fn normalize_base_url(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarise_groups_pushes_per_project_branch() {
        let events = vec![
            json!({
                "action_name": "pushed to",
                "_project_name": "team/myrepo",
                "push_data": { "ref": "feat/foo", "commit_count": 3 }
            }),
            json!({
                "action_name": "pushed to",
                "_project_name": "team/myrepo",
                "push_data": { "ref": "feat/foo", "commit_count": 1 }
            }),
            json!({
                "action_name": "pushed to",
                "_project_name": "team/otherrepo",
                "push_data": { "ref": "feat/foo", "commit_count": 2 }
            }),
        ];
        let groups = summarise(&events);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].project, "myrepo");
        assert_eq!(groups[0].lines.len(), 1);
        assert_eq!(groups[0].lines[0].text, "feat/foo (2 pushes, 4 commits)");
        assert_eq!(groups[1].project, "otherrepo");
        assert_eq!(groups[1].lines[0].text, "feat/foo (2 commits)");
    }

    #[test]
    fn summarise_pushes_first_then_other_actions_within_project() {
        let events = vec![
            json!({
                "action_name": "approved",
                "_project_name": "team/myrepo",
                "target_title": "x"
            }),
            json!({
                "action_name": "pushed to",
                "_project_name": "team/myrepo",
                "push_data": { "ref": "main", "commit_count": 1 }
            }),
        ];
        let groups = summarise(&events);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lines[0].icon, ICON_PUSH);
        assert_eq!(groups[0].lines[1].icon, ICON_APPROVE);
    }

    #[test]
    fn summarise_falls_back_to_numeric_project_id() {
        let events = vec![json!({
            "action_name": "approved",
            "project_id": 42,
            "target_title": "x"
        })];
        let groups = summarise(&events);
        assert_eq!(groups[0].project, "#42");
        assert_eq!(groups[0].lines[0].text, "Approved: x");
    }

    #[test]
    fn summarise_appends_mr_author_to_merge_lines() {
        let events = vec![json!({
            "action_name": "accepted",
            "_project_name": "team/myrepo",
            "_mr_author_username": "alice",
            "target_type": "MergeRequest",
            "target_title": "fix things"
        })];
        let groups = summarise(&events);
        assert_eq!(groups[0].lines[0].text, "Merged MR: fix things (by alice)");
    }

    #[test]
    fn summarise_collapses_comments_per_mr_with_author() {
        let events = vec![
            json!({
                "action_name": "commented on",
                "_project_name": "team/myrepo",
                "_mr_author_username": "alice",
                "target_type": "DiffNote",
                "target_title": "fix things",
                "note": { "noteable_type": "MergeRequest", "noteable_iid": 7 }
            }),
            json!({
                "action_name": "commented on",
                "_project_name": "team/myrepo",
                "_mr_author_username": "alice",
                "target_type": "DiffNote",
                "target_title": "fix things",
                "note": { "noteable_type": "MergeRequest", "noteable_iid": 7 }
            }),
        ];
        let groups = summarise(&events);
        assert_eq!(
            groups[0].lines[0].text,
            "Commented on MR: fix things (by alice) (2 comments)"
        );
    }

    #[test]
    fn normalize_strips_trailing_slash_and_prepends_https() {
        assert_eq!(normalize_base_url("gitlab.example.com"), "https://gitlab.example.com");
        assert_eq!(
            normalize_base_url("https://gitlab.example.com/"),
            "https://gitlab.example.com"
        );
        assert_eq!(
            normalize_base_url("http://gitlab.local/"),
            "http://gitlab.local"
        );
        assert_eq!(
            normalize_base_url("  https://gitlab.com  "),
            "https://gitlab.com"
        );
    }
}
