//! HTTPS certificate: self-signed (created on first start) or provided by the user.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub struct CertFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
    /// SHA-256 of the certificate (DER), lowercase hex.
    pub fingerprint: String,
    pub self_signed: bool,
}

/// Names the self-signed certificate covers: this machine's host name, `localhost`, loopback
/// and local interface addresses, plus anything given with `--name`.
pub fn default_names(extra: &[String]) -> Vec<String> {
    let mut names = vec!["localhost".to_owned(), "127.0.0.1".to_owned(), "::1".to_owned()];
    if let Some(h) = hostname() {
        names.push(h.clone());
        if !h.contains('.') {
            names.push(format!("{h}.local"));
        }
    }
    if let Ok(ifs) = if_addrs::get_if_addrs() {
        names.extend(ifs.into_iter().filter(|i| !i.is_loopback()).map(|i| i.ip().to_string()));
    }
    names.extend(extra.iter().cloned());
    names.sort();
    names.dedup();
    names
}

pub fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes at most buf.len() bytes into our buffer.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok().filter(|s| !s.is_empty())
}

/// Use the given certificate, or the self-signed one in `dir` (created if missing or if it
/// doesn't cover all `names`).
pub fn prepare(dir: &Path, own: Option<(PathBuf, PathBuf)>, names: &[String]) -> Result<CertFiles> {
    if let Some((cert, key)) = own {
        let fingerprint = fingerprint_of(&std::fs::read_to_string(&cert).with_context(|| format!("reading {}", cert.display()))?)?;
        return Ok(CertFiles { cert, key, fingerprint, self_signed: false });
    }
    std::fs::create_dir_all(dir)?;
    let cert = dir.join("cert.pem");
    let key = dir.join("key.pem");
    let names_file = dir.join("names.txt");
    let wanted = names.join("\n");
    let current = std::fs::read_to_string(&names_file).unwrap_or_default();
    let covered = current.lines().collect::<std::collections::HashSet<_>>();
    if !cert.exists() || !key.exists() || !names.iter().all(|n| covered.contains(n.as_str())) {
        let mut all: Vec<String> = covered.iter().map(|s| s.to_string()).chain(names.iter().cloned()).collect();
        all.sort();
        all.dedup();
        let mut params = rcgen::CertificateParams::new(all.clone())?;
        params.distinguished_name.push(rcgen::DnType::CommonName, format!("Remotium host {}", hostname().unwrap_or_default()));
        let signing_key = rcgen::KeyPair::generate()?;
        let c = params.self_signed(&signing_key)?;
        write_private(&key, signing_key.serialize_pem().as_bytes())?;
        std::fs::write(&cert, c.pem())?;
        std::fs::write(&names_file, all.join("\n"))?;
        tracing::info!(names = %all.join(", "), "created a self-signed certificate");
    } else if !wanted.is_empty() {
        tracing::debug!("reusing the self-signed certificate");
    }
    let fingerprint = fingerprint_of(&std::fs::read_to_string(&cert)?)?;
    Ok(CertFiles { cert, key, fingerprint, self_signed: true })
}

/// SHA-256 of the first certificate in a PEM file.
pub fn fingerprint_of(pem: &str) -> Result<String> {
    let begin = "-----BEGIN CERTIFICATE-----";
    let Some(start) = pem.find(begin) else { bail!("no certificate in PEM") };
    let rest = &pem[start + begin.len()..];
    let Some(end) = rest.find("-----END CERTIFICATE-----") else { bail!("unterminated certificate") };
    let b64: String = rest[..end].chars().filter(|c| !c.is_whitespace()).collect();
    let der = STANDARD.decode(b64).context("certificate is not valid base64")?;
    Ok(hex::encode(Sha256::digest(der)))
}

/// `AB:CD:…`, the way browsers show it.
pub fn pretty(fp: &str) -> String {
    fp.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).to_uppercase()).collect::<Vec<_>>().join(":")
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?.write_all(data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_once_and_regenerates_for_new_names() {
        let dir = tempfile::tempdir().unwrap();
        let a = prepare(dir.path(), None, &["localhost".into()]).unwrap();
        let b = prepare(dir.path(), None, &["localhost".into()]).unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.fingerprint.len(), 64);
        let c = prepare(dir.path(), None, &["localhost".into(), "dev.example".into()]).unwrap();
        assert_ne!(a.fingerprint, c.fingerprint);
        assert!(std::fs::read_to_string(dir.path().join("names.txt")).unwrap().contains("dev.example"));
        assert_eq!(pretty("ab01"), "AB:01");
    }
}
