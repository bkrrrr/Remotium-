//! The client extension, offered for download by the server itself.
//!
//! The server image contains the matching `remotium-client.vsix`, so users can install the
//! client from the same dashboard they launch from, with no extension store involved.

use crate::AppState;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc};

/// Where the server image puts the client package.
pub const DEFAULT_CLIENT_VSIX: &str = "/usr/share/remotium/remotium-client.vsix";
pub const VSIX_NAME: &str = "remotium-client.vsix";

pub struct ClientPackage {
    pub bytes: Bytes,
    pub sha256: String,
}

impl ClientPackage {
    /// Load the configured package, or the image default if it exists.
    pub fn load(configured: Option<&Path>) -> Option<Self> {
        let path = match configured {
            Some(p) => p.to_owned(),
            None => {
                let d = Path::new(DEFAULT_CLIENT_VSIX);
                if !d.exists() {
                    return None;
                }
                d.to_owned()
            }
        };
        match std::fs::read(&path) {
            Ok(b) => {
                let sha256 = hex::encode(Sha256::digest(&b));
                tracing::info!(path = %path.display(), bytes = b.len(), sha256, "serving the client package");
                Some(Self { bytes: b.into(), sha256 })
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), "client package not available: {e}");
                None
            }
        }
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "this server has no client package configured").into_response()
}

pub async fn vsix(State(state): State<Arc<AppState>>) -> Response {
    let Some(c) = &state.client else { return not_found() };
    (
        [
            (header::CONTENT_TYPE, "application/vsix".to_owned()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{VSIX_NAME}\"")),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
        ],
        c.bytes.clone(),
    )
        .into_response()
}

pub async fn vsix_sha256(State(state): State<Arc<AppState>>) -> Response {
    let Some(c) = &state.client else { return not_found() };
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], format!("{}  {VSIX_NAME}\n", c.sha256)).into_response()
}

/// Server URLs end up inside single-quoted shell/PowerShell strings.
fn script_safe(url: &str) -> bool {
    url.bytes().all(|b| b.is_ascii_graphic() && !matches!(b, b'\'' | b'"' | b'`' | b'$' | b'\\'))
}

fn script(state: &AppState, template: &str) -> Response {
    let Some(c) = &state.client else { return not_found() };
    let server = state.config.server.public_url.trim_end_matches('/');
    if !script_safe(server) {
        return (StatusCode::INTERNAL_SERVER_ERROR, "public_url contains characters not allowed in install scripts").into_response();
    }
    let body = template.replace("{{SERVER}}", server).replace("{{SHA256}}", &c.sha256);
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}

pub async fn install_sh(State(state): State<Arc<AppState>>) -> Response {
    script(&state, INSTALL_SH)
}

pub async fn install_ps1(State(state): State<Arc<AppState>>) -> Response {
    script(&state, INSTALL_PS1)
}

pub const INSTALL_SH: &str = r#"#!/bin/sh
# Installs Remotium Client into VSCodium, downloaded from {{SERVER}}.
#   curl -fsSL {{SERVER}}/install.sh | sh
# Set CODIUM=/path/to/codium if VSCodium is installed somewhere unusual.
set -eu
server='{{SERVER}}'
sha256='{{SHA256}}'

find_codium() {
    if [ -n "${CODIUM:-}" ]; then echo "$CODIUM"; return 0; fi
    if command -v codium >/dev/null 2>&1; then command -v codium; return 0; fi
    for c in \
        "/Applications/VSCodium.app/Contents/Resources/app/bin/codium" \
        "$HOME/Applications/VSCodium.app/Contents/Resources/app/bin/codium" \
        /usr/share/codium/bin/codium /opt/vscodium/bin/codium /snap/bin/codium \
        /var/lib/flatpak/exports/bin/com.vscodium.codium \
        "$HOME/.local/share/flatpak/exports/bin/com.vscodium.codium"; do
        if [ -x "$c" ]; then echo "$c"; return 0; fi
    done
    return 1
}

codium=$(find_codium) || {
    echo "VSCodium was not found. Install it from https://vscodium.com, or run with CODIUM=/path/to/codium." >&2
    exit 1
}
tmp=$(mktemp -d 2>/dev/null || mktemp -d -t remotium)
trap 'rm -rf "$tmp"' EXIT INT TERM
file="$tmp/remotium-client.vsix"

echo "Downloading Remotium Client from $server ..."
curl -fsSL -o "$file" "$server/downloads/remotium-client.vsix"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$file" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$file" | cut -d' ' -f1)
fi
if [ "$actual" != "$sha256" ]; then
    echo "Checksum mismatch: expected $sha256, got $actual. Not installing." >&2
    exit 1
fi

"$codium" --install-extension "$file" --force
echo
echo "Remotium Client is installed. Start VSCodium (or restart it): Remotium asks once to"
echo "finish its setup. Then open your environment from $server"
"#;

pub const INSTALL_PS1: &str = r#"# Installs Remotium Client into VSCodium, downloaded from {{SERVER}}.
#   irm {{SERVER}}/install.ps1 | iex
# Set $env:CODIUM to the path of codium.cmd if VSCodium is installed somewhere unusual.
$ErrorActionPreference = 'Stop'
$server = '{{SERVER}}'
$sha256 = '{{SHA256}}'

$codium = $env:CODIUM
if (-not $codium) {
    $cmd = Get-Command codium -ErrorAction SilentlyContinue
    if ($cmd) { $codium = $cmd.Source }
}
if (-not $codium) {
    foreach ($c in @("$env:LOCALAPPDATA\Programs\VSCodium\bin\codium.cmd", "$env:ProgramFiles\VSCodium\bin\codium.cmd")) {
        if (Test-Path $c) { $codium = $c; break }
    }
}
if (-not $codium) { throw 'VSCodium was not found. Install it from https://vscodium.com, or set $env:CODIUM to codium.cmd.' }

$file = Join-Path ([IO.Path]::GetTempPath()) ("remotium-client-" + [guid]::NewGuid() + ".vsix")
try {
    Write-Host "Downloading Remotium Client from $server ..."
    Invoke-WebRequest -UseBasicParsing -Uri "$server/downloads/remotium-client.vsix" -OutFile $file
    $actual = (Get-FileHash -Algorithm SHA256 -Path $file).Hash.ToLowerInvariant()
    if ($actual -ne $sha256) { throw "Checksum mismatch: expected $sha256, got $actual. Not installing." }
    & $codium --install-extension $file --force
    if ($LASTEXITCODE -ne 0) { throw "codium --install-extension failed ($LASTEXITCODE)" }
} finally {
    Remove-Item -Force -ErrorAction SilentlyContinue $file
}
Write-Host ""
Write-Host "Remotium Client is installed. Start VSCodium (or restart it): Remotium asks once to"
Write-Host "finish its setup. Then open your environment from $server"
"#;
