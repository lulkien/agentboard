//! `agentboard daemon`: the twelve MCP tools over streamable HTTP, one process, one
//! open board. The actor is the URL segment in `/mcp/<actor>`; the project is the
//! tool argument, else `?project=ID` on that URL, else the whole board.
//!
//! The port is also a browser-reachable write surface, so one guard runs before
//! every route: `Host` must be ours, an `Origin` must be ours, and `/mcp` answers
//! loopback peers only whatever the bind address.
//!
//! The dashboard is three embedded files (`dashboard/`). Every page path answers
//! the same `index.html`; `app.js` fetches the JSON API for that path and builds
//! the DOM. Stateless plain GET pages: no client router, no write route.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rust_embed::RustEmbed;

mod api;

use crate::server::{Board as McpBoard, RequestScope};
use crate::store::Board;

/// The dashboard files, compiled into the binary (also in debug builds).
#[derive(RustEmbed)]
#[folder = "dashboard/"]
struct Assets;

const CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

type Mcp = StreamableHttpService<McpBoard, NeverSessionManager>;

#[derive(Clone)]
struct App {
    mcp: Mcp,
    board: Arc<Board>,
}

#[cfg(test)]
pub(crate) fn epics_json_for_test(board: &Board, project: Option<u64>) -> serde_json::Value {
    api::epics_json_for_test(board, project)
}

/// Serve until the process is killed. `board` is the one open handle that every
/// request shares.
pub async fn serve(
    board: Board,
    address: SocketAddr,
    allow_non_loopback: bool,
) -> Result<(), String> {
    // The Host guard accepts only the bound address, `localhost` and `127.0.0.1`, so a
    // wildcard bind would answer 421 to every remote client. Name one address.
    if address.ip().is_unspecified() {
        return Err(format!(
            "daemon cannot bind the unspecified address {}; name one address, for example 192.168.1.50:8765",
            address.ip()
        ));
    }
    if !address.ip().is_loopback() && !allow_non_loopback {
        return Err(
            "daemon binding to a non-loopback address requires --allow-non-loopback".into(),
        );
    }
    let board = Arc::new(board);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|e| format!("daemon bind: {e}"))?;
    let bound = listener
        .local_addr()
        .map_err(|e| format!("daemon bound address: {e}"))?;
    let instance = board.metadata().map_err(|e| e.to_string())?.instance_id;
    println!(
        "agentboard daemon listening on http://{bound} board {} instance {instance}",
        board.path().display()
    );
    let app = router(board, bound);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .map_err(|e| format!("daemon: {e}"))
}

fn router(board: Arc<Board>, bound: SocketAddr) -> Router {
    // Stateless: a resident process must not accumulate sessions from seats that
    // die without closing one, and the tools never push notifications. The guard
    // below owns the Host check, so rmcp's own list is switched off.
    let server = McpBoard::shared(Arc::clone(&board), None);
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .disable_allowed_hosts();
    let mcp = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    Router::new()
        .route("/mcp/{actor}", any(mcp_request))
        .route("/", get(shell))
        .route("/epics", get(shell))
        .route("/tasks", get(shell))
        .route("/tasks/{id}", get(shell))
        .route("/feed", get(shell))
        .route("/assets/{name}", get(asset))
        .route("/api/health", get(health))
        .route("/api/board", get(api::board))
        .route("/api/epics", get(api::epics))
        .route("/api/tasks", get(api::tasks))
        .route("/api/tasks/{id}", get(api::task))
        .route("/api/feed", get(api::feed))
        .route("/api/projects", get(api::projects))
        .layer(middleware::from_fn(api_get_only))
        .layer(middleware::from_fn_with_state(bound, guard))
        .layer(middleware::from_fn(browser_headers))
        .with_state(App { mcp, board })
}

async fn shell() -> Response {
    file("index.html")
}

/// `index.html` is only reachable as a page, never as an asset.
async fn asset(Path(name): Path<String>) -> Response {
    if name == "index.html" {
        return StatusCode::NOT_FOUND.into_response();
    }
    file(&name)
}

fn file(name: &str) -> Response {
    let Some(found) = Assets::get(name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        _ => "application/octet-stream",
    };
    (
        [(header::CONTENT_TYPE, content_type)],
        found.data.into_owned(),
    )
        .into_response()
}

/// Everything a browser can load, guard refusals included, is uncacheable, never
/// sniffed, sends no referrer and may only load our own files. `/mcp` is for
/// agents, not browsers, and keeps rmcp's headers.
async fn browser_headers(request: Request, next: Next) -> Response {
    let browser = !request.uri().path().starts_with("/mcp");
    let mut response = next.run(request).await;
    if browser {
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
        headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
        headers.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
        headers.insert(header::CONTENT_SECURITY_POLICY, CSP.parse().unwrap());
    }
    response
}

async fn mcp_request(
    State(app): State<App>,
    Path(actor): Path<String>,
    mut request: Request,
) -> Response {
    if actor.trim().is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let project = match crate::routes::parse("/", request.uri().query().unwrap_or("")) {
        Ok(route) => route.filters.project,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid query").into_response(),
    };
    request
        .extensions_mut()
        .insert(RequestScope { actor, project });
    app.mcp.handle(request).await.map(Body::new)
}

async fn health(State(app): State<App>) -> Response {
    let read = app
        .board
        .metadata()
        .and_then(|metadata| Ok((metadata, app.board.cursor_now()?)));
    match read {
        Ok((metadata, cursor)) => axum::Json(serde_json::json!({
            "board": app.board.path().display().to_string(),
            "instance": metadata.instance_id,
            "cursor": cursor.encode(),
        }))
        .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

/// axum answers HEAD from a GET route; the API is GET only, so HEAD is 405 too.
async fn api_get_only(request: Request, next: Next) -> Response {
    if request.method() == Method::HEAD && request.uri().path().starts_with("/api/") {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    next.run(request).await
}

/// Runs before every route.
async fn guard(
    State(bound): State<SocketAddr>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| host_allowed(host, bound));
    if !host_ok {
        return (StatusCode::MISDIRECTED_REQUEST, "misdirected request").into_response();
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let ours = origin
            .to_str()
            .is_ok_and(|origin| origin_allowed(origin, bound));
        if !ours {
            return (StatusCode::FORBIDDEN, "foreign origin").into_response();
        }
    }
    if request.uri().path().starts_with("/mcp") && !is_loopback(peer.ip()) {
        return (StatusCode::FORBIDDEN, "mcp serves loopback peers only").into_response();
    }
    next.run(request).await
}

/// An `Origin` is ours when it is `http://` plus an allowed host. A browser omits
/// the port of the scheme default, so a bare host means port 80, which is ours
/// only when the daemon is bound to 80.
fn origin_allowed(origin: &str, bound: SocketAddr) -> bool {
    let Some(authority) = origin.strip_prefix("http://") else {
        return false;
    };
    let explicit_port = if authority.starts_with('[') {
        authority.contains("]:")
    } else {
        authority.contains(':')
    };
    (explicit_port || bound.port() == 80) && host_allowed(authority, bound)
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.to_canonical().is_loopback()
}

pub fn host_allowed(host: &str, bound_address: SocketAddr) -> bool {
    let host = host.trim();
    if host
        .parse::<SocketAddr>()
        .is_ok_and(|candidate| candidate == bound_address)
    {
        return true;
    }
    let bare_ip = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    if bare_ip
        .parse::<IpAddr>()
        .is_ok_and(|candidate| candidate == bound_address.ip())
    {
        return true;
    }
    let (name, port) = match host.rsplit_once(':') {
        Some((name, raw_port)) if !name.contains(':') => {
            let Ok(port) = raw_port.parse::<u16>() else {
                return false;
            };
            (name, Some(port))
        }
        _ => (host, None),
    };
    if port.is_some_and(|port| port != bound_address.port()) {
        return false;
    }
    name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_serves_loopback_peers_only() {
        for ip in ["127.0.0.1", "::1", "::ffff:127.0.0.1"] {
            assert!(is_loopback(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["192.168.1.9", "10.0.0.1", "2001:db8::1", "::ffff:10.0.0.1"] {
            assert!(!is_loopback(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn origin_port_must_be_ours() {
        let bound: SocketAddr = "127.0.0.1:8765".parse().unwrap();
        for ok in ["http://127.0.0.1:8765", "http://localhost:8765"] {
            assert!(origin_allowed(ok, bound), "{ok}");
        }
        for bad in [
            "http://localhost",
            "http://127.0.0.1",
            "http://localhost:80",
            "http://localhost:9",
            "https://localhost:8765",
            "null",
            "http://evil.example",
        ] {
            assert!(!origin_allowed(bad, bound), "{bad}");
        }
        let eighty: SocketAddr = "127.0.0.1:80".parse().unwrap();
        assert!(origin_allowed("http://localhost", eighty));
    }

    #[test]
    fn host_must_be_ours() {
        let bound: SocketAddr = "127.0.0.1:8765".parse().unwrap();
        for ok in ["127.0.0.1:8765", "localhost:8765", "LOCALHOST", "127.0.0.1"] {
            assert!(host_allowed(ok, bound), "{ok}");
        }
        for bad in ["evil.example", "localhost:9", "evil.example:8765", ""] {
            assert!(!host_allowed(bad, bound), "{bad}");
        }
    }
}
