//! `agentboard daemon`: the ten MCP tools over streamable HTTP, one process, one
//! open board. The actor is the URL segment in `/mcp/<actor>`; the project is the
//! tool argument, else `?project=ID` on that URL, else the whole board.
//!
//! The port is also a browser-reachable write surface, so one guard runs before
//! every route: `Host` must be ours, an `Origin` must be ours, and `/mcp` answers
//! loopback peers only whatever the bind address.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use crate::server::{Board as McpBoard, RequestScope};
use crate::store::Board;

type Mcp = StreamableHttpService<McpBoard, NeverSessionManager>;

#[derive(Clone)]
struct App {
    mcp: Mcp,
    board: Arc<Board>,
}

/// Serve until the process is killed. `board` is the one open handle that every
/// request shares.
pub async fn serve(
    board: Board,
    address: SocketAddr,
    allow_non_loopback: bool,
) -> Result<(), String> {
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
        .route("/api/health", get(health))
        .layer(middleware::from_fn_with_state(bound, guard))
        .with_state(App { mcp, board })
}

async fn mcp_request(
    State(app): State<App>,
    Path(actor): Path<String>,
    mut request: Request,
) -> Response {
    if actor.trim().is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let project = match project_param(request.uri().query()) {
        Ok(project) => project,
        Err(message) => return (StatusCode::BAD_REQUEST, message).into_response(),
    };
    request
        .extensions_mut()
        .insert(RequestScope { actor, project });
    app.mcp.handle(request).await.map(Body::new)
}

/// The `project` key of the MCP URL: absent is the whole board, anything but a
/// single numeric id is refused rather than guessed.
fn project_param(query: Option<&str>) -> Result<Option<u64>, &'static str> {
    let mut project = None;
    for pair in query
        .unwrap_or("")
        .split('&')
        .filter(|pair| !pair.is_empty())
    {
        if let Some(raw) = pair.strip_prefix("project=") {
            let id = raw
                .parse::<u64>()
                .map_err(|_| "project must be a numeric id")?;
            if project.replace(id).is_some() {
                return Err("project given twice");
            }
        }
    }
    Ok(project)
}

async fn health(State(app): State<App>) -> Response {
    match app.board.metadata() {
        Ok(metadata) => axum::Json(serde_json::json!({
            "board": app.board.path().display().to_string(),
            "instance": metadata.instance_id,
        }))
        .into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
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
    fn the_url_project_is_one_numeric_id_or_nothing() {
        assert_eq!(project_param(None), Ok(None));
        assert_eq!(project_param(Some("other=1")), Ok(None));
        assert_eq!(project_param(Some("project=7")), Ok(Some(7)));
        assert!(project_param(Some("project=x")).is_err());
        assert!(project_param(Some("project=")).is_err());
        assert!(project_param(Some("project=1&project=2")).is_err());
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
