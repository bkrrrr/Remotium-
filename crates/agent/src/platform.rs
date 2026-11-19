//! The `${os}` and `${arch}` values used in VSCodium REH asset names.

pub fn os() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        if std::path::Path::new("/etc/alpine-release").exists() || cfg!(target_env = "musl") && !glibc_present() {
            "alpine"
        } else {
            "linux"
        }
    } else {
        std::env::consts::OS
    }
}

fn glibc_present() -> bool {
    ["/lib/x86_64-linux-gnu/libc.so.6", "/lib/aarch64-linux-gnu/libc.so.6", "/lib64/libc.so.6", "/lib/libc.so.6", "/usr/lib/libc.so.6"]
        .iter()
        .any(|p| std::path::Path::new(p).exists())
}

pub fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "arm" => "armhf",
        "powerpc64" => "ppc64le",
        "riscv64" => "riscv64",
        "s390x" => "s390x",
        "loongarch64" => "loong64",
        other => other,
    }
}
