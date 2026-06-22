use crate::RELEASE_REPO;
use serde::Deserialize;

/// The rust target triple matching the current host, as used in ao-cli release
/// asset names (`ao-<version>-<target>.tar.gz`).
///
/// ao-cli publishes only glibc Linux, macOS, and MSVC Windows assets. A musl host would
/// be unable to run a downloaded `-gnu` binary and there is no musl asset to fetch, so we
/// report it as unsupported rather than handing back an incompatible target.
pub fn host_target() -> Result<&'static str, String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    // `OS`/`ARCH` cannot distinguish glibc from musl; use the avm build's own target env
    // as a proxy — a musl-built avm is the case that would silently fetch a broken binary.
    if os == "linux" && cfg!(target_env = "musl") {
        return Err(format!(
            "unsupported host platform: linux/{arch} (musl) — ao-cli publishes only \
             glibc Linux release assets"
        ));
    }
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        _ => Err(format!("unsupported host platform: {os}/{arch}")),
    }
}

pub fn tarball_name(version: &str, target: &str) -> String {
    format!("ao-{version}-{target}.tar.gz")
}

pub fn download_base(version: &str) -> String {
    format!("https://github.com/{RELEASE_REPO}/releases/download/{version}")
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
}

/// List available release tags via the GitHub API. Honors GITHUB_TOKEN for rate limits.
pub fn list_remote() -> Result<Vec<String>, String> {
    let url = format!("https://api.github.com/repos/{RELEASE_REPO}/releases?per_page=100");
    let mut req = ureq::get(&url)
        .set("User-Agent", "avm")
        .set("Accept", "application/vnd.github+json");
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
    }
    let resp = req
        .call()
        .map_err(|e| format!("GitHub API request failed: {e}"))?;
    let releases: Vec<GhRelease> = resp
        .into_json()
        .map_err(|e| format!("failed to parse GitHub API response: {e}"))?;
    Ok(releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .map(|r| r.tag_name)
        .collect())
}
