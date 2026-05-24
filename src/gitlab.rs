use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use reqwest::header::HeaderMap;
use serde::Deserialize;
use serde_json::Value;

/// One event from `/api/v4/events` kept as raw JSON — we don't know yet which
/// fields we'll want to summarise, so just hand the blob through and design
/// the summariser against real data in phase 4.
pub type RawEvent = Value;

#[derive(Debug, Clone, Deserialize)]
struct User {
    id: u64,
}

#[derive(Clone)]
pub struct Client {
    base_url: String,
    client: reqwest::Client,
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
            .user_agent("standup")
            .build()
            .context("building HTTP client")?;
        Ok(Self { base_url, client })
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

    /// Fetch all events for `user_id` on `date` (paginated).
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
        Ok(all)
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
