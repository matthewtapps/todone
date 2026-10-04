//! Xero sessions for the timesheet: parsed from a pasted cURL, or obtained by
//! running the Xero web app's own PKCE sign-in and redeeming the code here.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const AUTHORIZE: &str = "https://login.xero.com/identity/connect/authorize";
const TOKEN: &str = "https://identity.xero.com/connect/token";
// The Xero web app's public client. Its only registered redirect is on
// go.xero.com, so you paste the address the sign-in lands on.
// It refuses `offline_access`, so there's no refresh token: sessions last ~12 minutes.
const CLIENT_ID: &str = "xero_business_go";
const REDIRECT: &str = "https://go.xero.com/oidc/silent.html";
const SCOPE: &str = "openid profile email xero_frontend-apis xero_frontend-platform-apis";

#[derive(Clone)]
pub struct Session {
    pub token: String,
    pub shortcode: String,
    pub user_id: String,
    pub exp: u64,
}

/// A sign-in started by `start_login`, waiting for its redirect.
pub struct PendingLogin {
    verifier: String,
    state: String,
    shortcode: String,
}

fn claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}

fn session(token: String, shortcode: String) -> Option<Session> {
    let c = claims(&token)?;
    Some(Session {
        user_id: c["xero_userid"].as_str()?.to_string(),
        exp: c["exp"].as_u64()?,
        token,
        shortcode,
    })
}

pub fn valid_shortcode(s: &str) -> bool {
    s.len() > 1 && s.starts_with('!') && s[1..].chars().all(|c| c.is_ascii_alphanumeric())
}

/// Pull the bearer token and org shortcode out of a "Copy as cURL" paste.
pub fn parse_curl(text: &str) -> Option<Session> {
    // Firefox writes ! as \041 inside $'...' strings.
    let text = text.replace("\\041", "!");
    let lower = text.to_lowercase();
    let token_at = lower.find("bearer ")? + "bearer ".len();
    let token: String = text[token_at..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let code_at = ["shortcode:", "/app/"]
        .iter()
        .find_map(|k| lower.find(k).map(|i| i + k.len()))?;
    let shortcode: String = text[code_at..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '!')
        .collect();
    if !valid_shortcode(&shortcode) {
        return None;
    }
    session(token, shortcode)
}

fn random(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("OS randomness");
    URL_SAFE_NO_PAD.encode(b)
}

/// The sign-in URL to open, and the state needed to redeem its code.
pub fn start_login(shortcode: String) -> (String, PendingLogin) {
    let verifier = random(64);
    let state = random(16);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let url = reqwest::Url::parse_with_params(
        AUTHORIZE,
        [
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("state", &state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ],
    )
    .expect("static URL")
    .to_string();
    (url, PendingLogin { verifier, state, shortcode })
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

async fn token_request(http: &reqwest::Client, form: &[(&str, &str)]) -> Result<TokenResponse, String> {
    let body = reqwest::Url::parse_with_params("http://x/", form)
        .expect("static URL")
        .query()
        .unwrap_or_default()
        .to_string();
    let resp = http
        .post(TOKEN)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("couldn't reach Xero: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("Xero {}: {}", status.as_u16(), &text[..text.len().min(300)]));
    }
    serde_json::from_str(&text).map_err(|e| format!("unexpected token response: {e}"))
}

/// Redeem the code in the URL Xero redirected to after sign-in.
pub async fn finish_login(
    http: &reqwest::Client,
    pending: Option<PendingLogin>,
    redirected: &str,
) -> Result<Session, String> {
    let url = reqwest::Url::parse(redirected.trim())
        .map_err(|_| "Paste the full address of the page Xero sent you to.".to_string())?;
    let param = |k: &str| url.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
    if let Some(e) = param("error") {
        return Err(format!("Xero refused the sign-in: {e} {}", param("error_description").unwrap_or_default()));
    }
    let code = param("code").ok_or("That address has no sign-in code in it.")?;
    let pending = pending
        .filter(|p| param("state").as_deref() == Some(p.state.as_str()))
        .ok_or("That sign-in doesn't match the one todone started. Start the sign-in again.")?;
    let t = token_request(
        http,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", &code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", &pending.verifier),
        ],
    )
    .await?;
    session(t.access_token, pending.shortcode)
        .ok_or_else(|| "Xero's token had no user in it.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(claims: Value) -> String {
        format!("eyJhbGc.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn parses_firefox_curl() {
        let token = jwt(json!({ "xero_userid": "u1", "exp": 99 }));
        let text = format!(
            "curl 'https://go.xero.com/api/projects/x' \\\n  -H $'Referer: https://go.xero.com/app/\\0412sTwt/projects' \\\n  -H 'authorization: Bearer {token}' \\\n  -H $'xero-tenant-shortcode: \\0412sTwt'"
        );
        let s = parse_curl(&text).unwrap();
        assert_eq!(s.token, token);
        assert_eq!(s.shortcode, "!2sTwt");
        assert_eq!(s.user_id, "u1");
        assert_eq!(s.exp, 99);
    }

    #[test]
    fn rejects_paste_without_token() {
        assert!(parse_curl("curl 'https://go.xero.com/app/!2sTwt/'").is_none());
    }

    #[test]
    fn login_url_carries_pkce_challenge() {
        let (url, pending) = start_login("!abc".into());
        let url = reqwest::Url::parse(&url).unwrap();
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pending.verifier.as_bytes()));
        assert_eq!(q["code_challenge"], expected);
        assert_eq!(q["state"], pending.state);
        assert!(!q["scope"].contains("offline_access"));
    }

    #[tokio::test]
    async fn finish_rejects_mismatched_state() {
        let (_, pending) = start_login("!abc".into());
        let err = finish_login(
            &reqwest::Client::new(),
            Some(pending),
            "https://go.xero.com/oidc/silent.html?code=c&state=other",
        )
        .await
        .err()
        .unwrap();
        assert!(err.contains("doesn't match"));
    }
}
