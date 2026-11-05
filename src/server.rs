//! Local timesheet web app: serves the embedded SPA and proxies Xero's
//! internal Projects API using a session from the browser or a Xero sign-in.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    calendar::RawVevent,
    config, export, gitlab, timesheet,
    xero_auth::{self, PendingLogin, Session},
};

/// Override with `TODONE_TIMESHEET_PORT` to run a second todone alongside;
/// the Xero shortcode and sidebar state stay per port in the browser; timesheet data is shared on disk.
fn port() -> u16 {
    static PORT: OnceLock<u16> = OnceLock::new();
    *PORT.get_or_init(|| {
        std::env::var("TODONE_TIMESHEET_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8765)
    })
}
const XERO: &str = "https://go.xero.com/api/projects/";
const CALENDAR_TTL: Duration = Duration::from_secs(15 * 60);
const GITLAB_TTL: Duration = Duration::from_secs(30 * 60);

/// Set when the server fails to start, so the TUI can report it.
pub static START_ERROR: OnceLock<String> = OnceLock::new();

pub fn url() -> String {
    format!("http://127.0.0.1:{}/", port())
}

#[derive(rust_embed::Embed)]
#[folder = "web/dist/"]
#[allow_missing = true]
struct Assets;

#[derive(Default)]
struct Shared {
    session: Mutex<Option<Session>>,
    pending_login: Mutex<Option<PendingLogin>>,
    calendar: tokio::sync::Mutex<Option<(Instant, Option<Vec<RawVevent>>)>>,
    gitlab: tokio::sync::Mutex<GitlabState>,
    http: reqwest::Client,
}

#[derive(Default)]
struct GitlabState {
    /// The (instance, token, username) the client was built for; a change rebuilds it.
    key: Option<(String, String, String)>,
    client: Option<gitlab::Client>,
    user_id: Option<u64>,
    days: HashMap<NaiveDate, (Instant, Value)>,
}

type AppState = Arc<Shared>;

pub async fn serve() {
    let result = async {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port())).await?;
        let app = Router::new()
            .route("/api/todone", get(todone))
            .route("/api/gitlab", get(gitlab_day))
            .route("/api/session", get(get_session).post(post_session))
            .route("/api/xero-login", post(login_start))
            .route("/api/xero-login/finish", post(login_finish))
            .route("/api/xero/{*path}", any(proxy))
            .route("/api/timesheet/settings", get(get_settings).put(put_settings))
            .route("/api/timesheet/{month}", get(get_month).put(put_month))
            .fallback(asset)
            .layer(middleware::from_fn(guard))
            .with_state(AppState::default());
        axum::serve(listener, app).await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = result {
        let _ = START_ERROR.set(format!("timesheet server on port {}: {e}", port()));
    }
}

fn error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

/// The proxy acts with your Xero session, so refuse DNS-rebound hosts, and
/// require a custom header on writes so cross-site forms can't reach it.
async fn guard(req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok());
    let host_ok = host.is_some_and(|h| {
        h.strip_prefix("127.0.0.1:").or_else(|| h.strip_prefix("localhost:")) == Some(&port().to_string())
    });
    let header_ok = req.method() == Method::GET
        || req.headers().get("x-timesheet").is_some_and(|v| v == "1");
    if !host_ok || !header_ok {
        return error(StatusCode::FORBIDDEN, "forbidden");
    }
    next.run(req).await
}

async fn asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = Assets::get(path).or_else(|| Assets::get("index.html"));
    match file {
        Some(f) => {
            let mime = f.metadata.mimetype().to_string();
            ([(header::CONTENT_TYPE, mime)], f.data).into_response()
        }
        None => error(
            StatusCode::NOT_FOUND,
            "the web app isn't built; run `npm ci && npm run build` in web/",
        ),
    }
}

#[derive(Deserialize)]
struct Range {
    from: NaiveDate,
    to: NaiveDate,
}

async fn todone(State(s): State<AppState>, Query(r): Query<Range>) -> Response {
    let mut cache = s.calendar.lock().await;
    let stale = cache.as_ref().is_none_or(|(at, _)| at.elapsed() > CALENDAR_TTL);
    if stale {
        match export::fetch_calendar().await {
            Ok(raws) => *cache = Some((Instant::now(), raws)),
            Err(e) => return error(StatusCode::BAD_GATEWAY, format!("calendar: {e:#}")),
        }
    }
    let raws = cache.as_ref().and_then(|(_, r)| r.as_deref());
    match export::days(r.from, r.to, raws) {
        Ok(days) => Json(days).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct Day {
    date: NaiveDate,
}

#[derive(Deserialize)]
struct PutDoc {
    version: u64,
    #[serde(flatten)]
    doc: Map<String, Value>,
}

fn with_version(version: u64, mut doc: Map<String, Value>) -> Value {
    doc.insert("version".into(), version.into());
    Value::Object(doc)
}

/// A GET when `put` is None; a stale PUT gets 409 with what's on disk.
fn timesheet_doc(path: anyhow::Result<std::path::PathBuf>, put: Option<PutDoc>) -> Response {
    let path = match path {
        Ok(p) => p,
        Err(e) => return error(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let result = match put {
        None => timesheet::read(&path).map(|(v, d)| (StatusCode::OK, with_version(v, d))),
        Some(p) => timesheet::put(&path, p.version, p.doc).map(|r| match r {
            timesheet::Put::Saved(v) => (StatusCode::OK, json!({ "version": v })),
            timesheet::Put::Stale(v, d) => (StatusCode::CONFLICT, with_version(v, d)),
        }),
    };
    match result {
        Ok((status, body)) => (status, Json(body)).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

// File locks block, so keep them off the async workers.
async fn blocking_doc(path: anyhow::Result<std::path::PathBuf>, put: Option<PutDoc>) -> Response {
    tokio::task::spawn_blocking(move || timesheet_doc(path, put))
        .await
        .unwrap_or_else(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_settings() -> Response {
    blocking_doc(timesheet::dir().map(|d| timesheet::settings_path(&d)), None).await
}

async fn put_settings(Json(p): Json<PutDoc>) -> Response {
    blocking_doc(timesheet::dir().map(|d| timesheet::settings_path(&d)), Some(p)).await
}

async fn get_month(Path(month): Path<String>) -> Response {
    blocking_doc(timesheet::dir().and_then(|d| timesheet::month_path(&d, &month)), None).await
}

async fn put_month(Path(month): Path<String>, Json(p): Json<PutDoc>) -> Response {
    blocking_doc(timesheet::dir().and_then(|d| timesheet::month_path(&d, &month)), Some(p)).await
}

fn gitlab_kind(icon: &str) -> &'static str {
    match icon {
        gitlab::ICON_PUSH => "push",
        gitlab::ICON_COMMENT => "comment",
        gitlab::ICON_APPROVE => "approve",
        gitlab::ICON_OPEN => "open",
        gitlab::ICON_MERGE => "merge",
        gitlab::ICON_CLOSE => "close",
        _ => "other",
    }
}

/// One day of GitLab activity, summarised by project as in the TUI's context pane.
async fn gitlab_day(State(s): State<AppState>, Query(q): Query<Day>) -> Response {
    let cfg = match config::default_path().and_then(|p| config::load(&p)) {
        Ok(c) => c.gitlab,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    };
    if !cfg.enabled || [&cfg.instance_url, &cfg.token, &cfg.username].iter().any(|v| v.trim().is_empty()) {
        return Json(json!({ "enabled": false })).into_response();
    }

    let mut g = s.gitlab.lock().await;
    let key = (cfg.instance_url.clone(), cfg.token.clone(), cfg.username.clone());
    if g.key.as_ref() != Some(&key) {
        *g = GitlabState::default();
        match gitlab::Client::new(&cfg.instance_url, &cfg.token) {
            Ok(c) => g.client = Some(c),
            Err(e) => return error(StatusCode::BAD_GATEWAY, format!("GitLab: {e:#}")),
        }
        g.key = Some(key);
    }
    if let Some((at, v)) = g.days.get(&q.date)
        && at.elapsed() < GITLAB_TTL
    {
        return Json(v.clone()).into_response();
    }
    let Some(client) = g.client.clone() else {
        return error(StatusCode::BAD_GATEWAY, "GitLab client unavailable");
    };
    let user = match g.user_id {
        Some(u) => u,
        None => match client.resolve_user(&cfg.username).await {
            Ok(u) => *g.user_id.insert(u),
            Err(e) => return error(StatusCode::BAD_GATEWAY, format!("GitLab: {e:#}")),
        },
    };
    let events = match client.fetch_events(user, q.date).await {
        Ok(e) => e,
        Err(e) => return error(StatusCode::BAD_GATEWAY, format!("GitLab: {e:#}")),
    };
    let groups: Vec<Value> = gitlab::summarise(&events)
        .into_iter()
        .map(|group| {
            let lines: Vec<Value> = group
                .lines
                .iter()
                .map(|l| json!({ "kind": gitlab_kind(l.icon), "text": l.text }))
                .collect();
            json!({ "project": group.project, "lines": lines })
        })
        .collect();
    let v = json!({ "enabled": true, "groups": groups });
    g.days.insert(q.date, (Instant::now(), v.clone()));
    Json(v).into_response()
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn live_session(s: &Shared) -> Option<Session> {
    s.session.lock().unwrap().clone().filter(|x| x.exp > now())
}

fn session_json(x: &Session) -> Json<Value> {
    Json(json!({ "userId": x.user_id, "exp": x.exp, "shortcode": x.shortcode }))
}

async fn get_session(State(s): State<AppState>) -> Response {
    match live_session(&s) {
        Some(x) => session_json(&x).into_response(),
        None => error(StatusCode::UNAUTHORIZED, "not connected to Xero"),
    }
}

fn set_session(s: &Shared, x: Session) -> Response {
    let body = session_json(&x);
    *s.session.lock().unwrap() = Some(x);
    body.into_response()
}

#[derive(Deserialize)]
struct Paste {
    text: String,
}

async fn post_session(State(s): State<AppState>, Json(p): Json<Paste>) -> Response {
    match xero_auth::parse_curl(&p.text) {
        Some(x) => set_session(&s, x),
        None => error(
            StatusCode::BAD_REQUEST,
            "paste a 'Copy as cURL' of a Xero /api/projects request",
        ),
    }
}

#[derive(Deserialize)]
struct StartLogin {
    shortcode: String,
}

async fn login_start(State(s): State<AppState>, Json(b): Json<StartLogin>) -> Response {
    let shortcode = b.shortcode.trim().to_string();
    if !xero_auth::valid_shortcode(&shortcode) {
        return error(
            StatusCode::BAD_REQUEST,
            "Enter your organisation code: the !xxxxx part of your Xero address.",
        );
    }
    let (url, pending) = xero_auth::start_login(shortcode);
    *s.pending_login.lock().unwrap() = Some(pending);
    Json(json!({ "url": url })).into_response()
}

async fn login_finish(State(s): State<AppState>, Json(p): Json<Paste>) -> Response {
    let pending = s.pending_login.lock().unwrap().take();
    match xero_auth::finish_login(&s.http, pending, &p.text).await {
        Ok(x) => set_session(&s, x),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}

async fn proxy(
    State(s): State<AppState>,
    method: Method,
    Path(path): Path<String>,
    uri: Uri,
    body: Bytes,
) -> Response {
    let Some(session) = live_session(&s) else {
        return error(StatusCode::UNAUTHORIZED, "Xero token expired");
    };
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let mut req = s
        .http
        .request(method.clone(), format!("{XERO}{path}{query}"))
        .bearer_auth(&session.token)
        .header("xero-tenant-shortcode", &session.shortcode)
        .header(header::ACCEPT, "application/json");
    if matches!(method, Method::POST | Method::PUT) {
        req = req.header(header::CONTENT_TYPE, "application/json").body(body);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_GATEWAY, format!("Xero request failed: {e}")),
    };
    let status = resp.status();
    // A create's Location ends in the new entry's ID.
    let location = resp.headers().get(header::LOCATION).cloned();
    let bytes = resp.bytes().await.unwrap_or_default();
    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes[..bytes.len().min(500)]).into_owned();
        return error(status, format!("Xero {}: {text}", status.as_u16()));
    }
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    if let Some(l) = location {
        headers.insert(header::LOCATION, l);
    }
    (status, headers, bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "todone-server-test-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    async fn body(r: Response) -> Value {
        serde_json::from_slice(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    fn put_body(v: Value) -> PutDoc {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn stale_put_is_409_with_disk_contents() {
        let path = tempdir().join("2026-10.json");
        let ok = timesheet_doc(Ok(path.clone()), Some(put_body(json!({"version": 0, "rows": {"disk": []}}))));
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(body(ok).await, json!({"version": 1}));

        let stale = timesheet_doc(Ok(path.clone()), Some(put_body(json!({"version": 0, "rows": {"mine": []}}))));
        assert_eq!(stale.status(), StatusCode::CONFLICT);
        assert_eq!(body(stale).await, json!({"version": 1, "rows": {"disk": []}}));

        let get = timesheet_doc(Ok(path), None);
        assert_eq!(body(get).await, json!({"version": 1, "rows": {"disk": []}}));
    }

    #[tokio::test]
    async fn bad_month_is_400() {
        let r = timesheet_doc(timesheet::month_path(&tempdir(), "2026-13"), None);
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }
}
