// Release builds embed Remotium Client so the host can offer it for download:
//   REMOTIUM_CLIENT_VSIX=/path/to/remotium-client.vsix cargo build --release
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=REMOTIUM_CLIENT_VSIX");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("client_vsix.rs");
    let code = match env::var("REMOTIUM_CLIENT_VSIX") {
        Ok(p) if !p.is_empty() => {
            // Relative paths are taken from the workspace root (build scripts run in the crate dir).
            let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
            let path = if PathBuf::from(&p).is_absolute() { PathBuf::from(&p) } else { root.join(&p) };
            let abs = fs::canonicalize(&path).unwrap_or_else(|e| panic!("REMOTIUM_CLIENT_VSIX={p}: {e}"));
            println!("cargo:rerun-if-changed={}", abs.display());
            format!("pub const EMBEDDED_CLIENT: Option<&[u8]> = Some(include_bytes!({:?}));\n", abs.display().to_string())
        }
        _ => "pub const EMBEDDED_CLIENT: Option<&[u8]> = None;\n".to_owned(),
    };
    fs::write(out, code).unwrap();
}
