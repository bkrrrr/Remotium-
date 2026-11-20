//! Server-rendered dashboard. No JavaScript.

use crate::{
    control::{check_same_site, create_launch, visible_envs},
    proxyauth::ProxyUser,
    AppState,
};
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Form,
};
use serde::Deserialize;
use std::sync::Arc;

const STYLE: &str = r#"
:root { color-scheme: light dark; --fg:#1d1d1f; --muted:#6e6e73; --bg:#fafafa; --card:#fff; --line:#e3e3e6; --ok:#1a7f37; --off:#9a6700; --accent:#2f6fde; }
@media (prefers-color-scheme: dark) { :root { --fg:#ececec; --muted:#a0a0a5; --bg:#161618; --card:#1f1f22; --line:#333338; --ok:#3fb950; --off:#d29922; --accent:#6ea0ff; } }
* { box-sizing: border-box; }
body { margin:0; font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif; color:var(--fg); background:var(--bg); }
main { max-width: 52rem; margin: 0 auto; padding: 2rem 1rem 4rem; }
header { display:flex; justify-content:space-between; align-items:baseline; gap:1rem; flex-wrap:wrap; margin-bottom:1.5rem; }
h1 { font-size:1.4rem; margin:0; }
.muted { color:var(--muted); font-size:.9rem; }
.env { background:var(--card); border:1px solid var(--line); border-radius:10px; padding:1rem 1.25rem; margin-bottom:1rem; }
.env h2 { font-size:1.1rem; margin:0; display:flex; align-items:center; gap:.5rem; }
.dot { width:.6rem; height:.6rem; border-radius:50%; display:inline-block; }
.on { background:var(--ok); } .offl { background:var(--off); }
ul { list-style:none; padding:0; margin:.75rem 0 0; }
li { display:flex; justify-content:space-between; align-items:center; gap:1rem; padding:.5rem 0; border-top:1px solid var(--line); flex-wrap:wrap; }
code { font-size:.85rem; }
button, .btn { font:inherit; color:#fff; background:var(--accent); border:0; border-radius:6px; padding:.35rem .8rem; cursor:pointer; text-decoration:none; display:inline-block; }
button[disabled] { opacity:.45; cursor:not-allowed; }
details { margin-top:2rem; } pre { background:var(--card); border:1px solid var(--line); border-radius:6px; padding:.75rem; overflow-x:auto; }
"#;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{STYLE}</style></head><body><main>{body}</main></body></html>",
        esc(title)
    ))
}

pub async fn index(State(state): State<Arc<AppState>>, ProxyUser(user): ProxyUser) -> Response {
    let envs = visible_envs(&state, &user);
    let mut body = format!(
        "<header><h1>Remotium</h1><span class=\"muted\">Signed in as {}</span></header>",
        esc(&user)
    );
    if envs.is_empty() {
        body.push_str("<p>You don't have access to any environment yet. Ask an administrator to add a grant for your account.</p>");
    }
    for e in &envs {
        let status = match &e.agent {
            Some(a) => format!(
                "<span class=\"dot on\"></span> online <span class=\"muted\">· {} · agent {} · {} open connection(s)</span>",
                esc(&a.arch),
                esc(&a.version),
                e.open_streams
            ),
            None => "<span class=\"dot offl\"></span> offline".to_owned(),
        };
        body.push_str(&format!(
            "<section class=\"env\"><h2>{}</h2><div class=\"muted\"><code>{}</code> · {status}</div><ul>",
            esc(&e.name),
            esc(&e.id)
        ));
        if e.projects.is_empty() {
            body.push_str("<li class=\"muted\">No projects available.</li>");
        }
        for p in &e.projects {
            body.push_str(&format!(
                "<li><span>{} <span class=\"muted\"><code>{}</code></span></span>\
                 <form method=\"post\" action=\"launch\"><input type=\"hidden\" name=\"env\" value=\"{}\">\
                 <input type=\"hidden\" name=\"project\" value=\"{}\"><button{}>Open in VSCodium</button></form></li>",
                esc(&p.name),
                esc(&p.path),
                esc(&e.id),
                esc(&p.name),
                if e.online { "" } else { " disabled title=\"Environment is offline\"" }
            ));
        }
        body.push_str("</ul></section>");
    }
    body.push_str(&setup_help(&state));
    let mut r = page("Remotium", &body).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    r
}

/// "Get the client" help. Open by default until the visitor has launched something.
fn setup_help(state: &AppState) -> String {
    let install = if state.config.server.client_on_open_vsx {
        "<li><a class=\"btn\" href=\"vscodium:extension/remotium.client\">Install Remotium Client</a> \
         <span class=\"muted\">(opens VSCodium; or search <b>Remotium</b> in its Extensions view)</span></li>"
            .to_owned()
    } else if state.client.is_some() {
        "<li><a class=\"btn\" href=\"downloads/remotium-client.vsix\" download>Download Remotium Client</a>, \
         then drag the file onto VSCodium's Extensions view (or <b>Extensions: Install from VSIX…</b>).</li>"
            .to_owned()
    } else {
        "<li>Install <b>Remotium Client</b> (<code>remotium.client</code>) in VSCodium. \
         <span class=\"muted\">This server has no download configured; ask your administrator for the file.</span></li>"
            .to_owned()
    };
    format!(
        r#"<details open><summary>Get the client (once per computer)</summary>
<ol>
{install}
<li>Run <b>Preferences: Configure Runtime Arguments</b>, add the line below, then restart VSCodium:
<pre>"enable-proposed-api": ["remotium.client"]</pre></li>
<li>Click <b>Open in VSCodium</b> above. The link is valid for 60 seconds and works once.</li>
</ol></details>"#
    )
}

#[derive(Deserialize)]
pub struct LaunchForm {
    env: String,
    project: String,
}

pub async fn launch(
    State(state): State<Arc<AppState>>,
    ProxyUser(user): ProxyUser,
    headers: HeaderMap,
    Form(f): Form<LaunchForm>,
) -> Response {
    if let Err(e) = check_same_site(&headers, &state.config.server.public_url) {
        return e.into_response();
    }
    match create_launch(&state, &user, &f.env, &f.project) {
        Ok(l) => {
            let uri = esc(&l.uri);
            let body = format!(
                "<h1>Opening VSCodium…</h1>\
                 <p>If nothing happens, <a class=\"btn\" href=\"{uri}\">open the link</a>. It works once and expires in {} seconds.</p>\
                 <p><a href=\"./\">Back to environments</a></p>",
                l.expires_in
            );
            let mut r = page("Opening VSCodium", &body).into_response();
            r.headers_mut().insert(header::REFRESH, format!("0; url={}", l.uri).parse().unwrap_or(header::HeaderValue::from_static("0")));
            r.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
            r
        }
        Err(e) => (StatusCode::FORBIDDEN, page("Not allowed", &format!("<h1>Not allowed</h1><p>{}</p><p><a href=\"./\">Back</a></p>", esc(&e.2)))).into_response(),
    }
}
