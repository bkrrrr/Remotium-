//! Browser access: pairing page, device cookie, and a reverse proxy (HTTP + WebSocket) to the
//! VSCodium web server.
//!
//! The device cookie is the credential: every request needs it. The VSCodium server's
//! connection token is also given to paired browsers (cookie `vscode-tkn`), because the stock
//! workbench reads it in JavaScript and sends it in its protocol handshake. On its own it opens
//! nothing: the server listens on 127.0.0.1 and is reachable only through this proxy.

use crate::{state::{DeviceKind, PairError}, AppState};
use axum::{
    body::Body,
    extract::{Form, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use std::sync::Arc;

pub const COOKIE: &str = "remotium_device";

const STYLE: &str = "body{font:15px/1.5 system-ui,sans-serif;max-width:34rem;margin:4rem auto;padding:0 1rem;color:#1d1d1f;background:#fafafa}\
@media(prefers-color-scheme:dark){body{color:#ececec;background:#161618}}\
h1{font-size:1.4rem}code{font-size:.9rem}input{font:inherit;padding:.3rem .5rem}\
button{font:inherit;color:#fff;background:#2f6fde;border:0;border-radius:6px;padding:.4rem 1rem;cursor:pointer}";

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{STYLE}</style></head><body>{body}</body></html>",
        esc(title)
    ))
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

#[derive(Deserialize)]
pub struct PairQuery {
    #[serde(default)]
    code: String,
}

/// `GET /pair?code=…`: a confirmation page. The code is only used by the POST, so link
/// previews and prefetchers can't burn it.
pub async fn pair_page(State(state): State<Arc<AppState>>, Query(q): Query<PairQuery>) -> Response {
    let body = format!(
        "<h1>Pair this browser with {name}</h1>\
         <p>After pairing, this browser can open <code>{folder}</code> on this machine.</p>\
         <form method=\"post\" action=\"pair\"><input type=\"hidden\" name=\"code\" value=\"{code}\">\
         <p><label>Name for this browser <input name=\"name\" value=\"Browser\" maxlength=\"64\"></label></p>\
         <p><button>Pair</button></p></form>{client}",
        client = client_hint(&state),
        name = esc(&state.name),
        folder = esc(&state.folder),
        code = esc(&q.code),
    );
    no_store(page("Pair this browser", &body).into_response())
}

#[derive(Deserialize)]
pub struct PairForm {
    code: String,
    #[serde(default)]
    name: String,
}

pub async fn pair_submit(State(state): State<Arc<AppState>>, headers: HeaderMap, Form(f): Form<PairForm>) -> Response {
    let name = if f.name.trim().is_empty() { "Browser".to_owned() } else { f.name.trim().to_owned() };
    match state.store.redeem(&f.code, DeviceKind::Browser, &name) {
        Ok(Ok((d, token))) => {
            tracing::info!(device = d.id, name = d.name, "paired a browser");
            let secure = if state.secure_cookies(&headers) { "; Secure" } else { "" };
            let cookie = format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=34560000{secure}");
            let mut r = Redirect::to(&format!("/?folder={}", pct(&state.folder))).into_response();
            r.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());
            r
        }
        Ok(Err(e)) => {
            let why = if e == PairError::Expired { "This pairing link has expired." } else { "This pairing link is unknown or was already used." };
            no_store((StatusCode::UNAUTHORIZED, page("Pairing failed", &format!("<h1>Pairing failed</h1><p>{why}</p>{AGAIN}"))).into_response())
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Where desktop VSCodium users get the extension.
fn client_hint(state: &AppState) -> String {
    if state.client.is_some() {
        format!(
            "<hr><p>Using desktop VSCodium? <a href=\"{}\" download>Download Remotium Client</a>, drag it onto VSCodium's \
             Extensions view, then open the VSCodium link printed by <code>remotium-host pair</code>.</p>",
            crate::client::PATH.trim_start_matches('/')
        )
    } else {
        "<hr><p>Using desktop VSCodium? Install <b>Remotium Client</b> (<code>remotium.client</code>), then open the \
         VSCodium link printed by <code>remotium-host pair</code>.</p>"
            .to_owned()
    }
}

const AGAIN: &str = "<p>On the host, run <code>remotium-host pair</code> and open the new link.</p>";

fn no_store(mut r: Response) -> Response {
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn device_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.to_owned())
}

/// Everything else: requires a paired browser, then goes to the VSCodium web server.
pub async fn proxy(State(state): State<Arc<AppState>>, mut req: Request) -> Response {
    if device_cookie(req.headers()).and_then(|t| state.store.check(&t)).is_none() {
        let body = format!("<h1>This browser isn't paired</h1>{AGAIN}");
        return no_store((StatusCode::UNAUTHORIZED, page("Not paired", &body)).into_response());
    }
    let web = state.web.clone();
    if let Err(e) = state.manager.ensure(&web).await {
        tracing::warn!("web server unavailable: {}", e.message);
        return (StatusCode::SERVICE_UNAVAILABLE, page("Unavailable", &format!("<h1>The editor isn't available</h1><p>{}</p>", esc(&e.message)))).into_response();
    }
    let (port, token, guard) = match state.manager.endpoint(&web.commit).await {
        Ok(e) => e,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, e.message).into_response(),
    };
    let has_token = req
        .headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .any(|kv| kv.trim() == format!("vscode-tkn={token}"));
    let secure = state.secure_cookies(req.headers());
    let upgrade = req.headers().get(header::UPGRADE).is_some();
    let client_upgrade = upgrade.then(|| hyper::upgrade::on(&mut req));

    // Rewrite the request for the local server.
    let (mut parts, body) = req.into_parts();
    // Browsers speak HTTP/2 over TLS; the local server speaks HTTP/1.1 and needs a Host header
    // (it builds the workbench's server address from it).
    if !parts.headers.contains_key(header::HOST) {
        if let Some(a) = parts.uri.authority().map(|a| a.as_str().to_owned()) {
            parts.headers.insert(header::HOST, HeaderValue::from_str(&a).unwrap_or(HeaderValue::from_static("localhost")));
        }
    }
    parts.version = axum::http::Version::HTTP_11;
    let path = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/").to_owned();
    parts.uri = path.parse().unwrap_or_else(|_| "/".parse().unwrap());
    let others: Vec<String> = parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|kv| !kv.is_empty() && !kv.starts_with(&format!("{COOKIE}=")) && !kv.starts_with("vscode-tkn="))
        .map(str::to_owned)
        .collect();
    parts.headers.remove(header::COOKIE);
    let mut cookie = others.join("; ");
    if !cookie.is_empty() {
        cookie.push_str("; ");
    }
    cookie.push_str(&format!("vscode-tkn={token}"));
    parts.headers.insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
    if !upgrade {
        for h in ["connection", "keep-alive", "proxy-connection", "te", "trailer", "upgrade"] {
            parts.headers.remove(h);
        }
    }
    let upstream_req = Request::from_parts(parts, body);

    let stream = match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("editor server: {e}")).into_response(),
    };
    let (mut sender, conn) = match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
        Ok(x) => x,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("editor server: {e}")).into_response(),
    };
    tokio::spawn(async move {
        if let Err(e) = conn.with_upgrades().await {
            tracing::debug!("proxy connection: {e}");
        }
    });
    let mut resp = match sender.send_request(upstream_req).await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("editor server: {e}")).into_response(),
    };

    if resp.status() == StatusCode::SWITCHING_PROTOCOLS {
        if let Some(client_upgrade) = client_upgrade {
            let server_upgrade = hyper::upgrade::on(&mut resp);
            tokio::spawn(async move {
                let _guard = guard;
                match tokio::try_join!(client_upgrade, server_upgrade) {
                    Ok((c, s)) => {
                        let (mut c, mut s) = (TokioIo::new(c), TokioIo::new(s));
                        let _ = tokio::io::copy_bidirectional(&mut c, &mut s).await;
                    }
                    Err(e) => tracing::debug!("websocket upgrade failed: {e}"),
                }
            });
        }
        let (parts, _) = resp.into_parts();
        return Response::from_parts(parts, Body::empty());
    }
    let (mut parts, body) = resp.into_parts();
    for h in ["connection", "keep-alive", "transfer-encoding"] {
        parts.headers.remove(h);
    }
    // The server sets vscode-tkn when it sees ?tkn=; the browser never needs it.
    let keep: Vec<HeaderValue> = parts
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter(|v| !v.to_str().unwrap_or_default().starts_with("vscode-tkn="))
        .cloned()
        .collect();
    parts.headers.remove(header::SET_COOKIE);
    for v in keep {
        parts.headers.append(header::SET_COOKIE, v);
    }
    if !has_token {
        // Readable by the workbench's JavaScript on purpose (see the module docs).
        let c = format!("vscode-tkn={token}; Path=/; SameSite=Strict{}", if secure { "; Secure" } else { "" });
        parts.headers.append(header::SET_COOKIE, HeaderValue::from_str(&c).unwrap());
    }
    let body = Body::new(body);
    drop(guard);
    Response::from_parts(parts, body)
}
