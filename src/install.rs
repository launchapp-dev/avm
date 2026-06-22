use crate::remote::{download_base, host_target, tarball_name};
use crate::{animus_exe_name, normalize_checked, version_binary_path, version_install_dir};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use tar::Archive;

/// Download, verify against the published SHA256SUMS.txt, and unpack a version
/// into `~/.avm/versions/<version>/`. Idempotent: a present binary is a no-op.
pub fn install(version_raw: &str) -> Result<(), String> {
    let version =
        normalize_checked(version_raw).ok_or_else(|| format!("invalid version `{version_raw}`"))?;
    let target = host_target()?;
    let dest = version_install_dir(&version);

    if version_binary_path(&version).is_file() {
        println!("{version} already installed at {}", dest.display());
        return Ok(());
    }

    let base = download_base(&version);
    let archive_name = tarball_name(&version, target);
    let archive_url = format!("{base}/{archive_name}");
    let sums_url = format!("{base}/SHA256SUMS.txt");

    eprintln!("avm: downloading {archive_url}");
    let archive_bytes = http_get_bytes(&archive_url)
        .map_err(|e| format!("failed to download {archive_name}: {e}"))?;

    eprintln!("avm: verifying SHA256");
    let sums = http_get_string(&sums_url)
        .map_err(|e| format!("failed to download SHA256SUMS.txt: {e}"))?;
    let expected = expected_hash(&sums, &archive_name)
        .ok_or_else(|| format!("{archive_name} not listed in SHA256SUMS.txt for {version}"))?;
    let actual = sha256_hex(&archive_bytes);
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(format!(
            "checksum mismatch for {archive_name}: expected {expected}, got {actual}"
        ));
    }

    eprintln!("avm: unpacking into {}", dest.display());
    let tmp = unpack_tmp_dir(&dest);
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    std::fs::create_dir_all(&tmp).map_err(|e| format!("mkdir {}: {e}", tmp.display()))?;

    unpack_flatten(&archive_bytes, &tmp)?;

    if !tmp.join(animus_exe_name()).is_file() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!(
            "unpacked archive did not contain an `{}` binary",
            animus_exe_name()
        ));
    }

    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| format!("rm {}: {e}", dest.display()))?;
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    std::fs::rename(&tmp, &dest).map_err(|e| format!("install rename failed: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = version_binary_path(&version);
        if let Ok(meta) = std::fs::metadata(&bin) {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o755);
            let _ = std::fs::set_permissions(&bin, perms);
        }
    }

    println!("installed {version} -> {}", dest.display());
    Ok(())
}

/// A version-specific staging directory adjacent to `dest`, e.g.
/// `.../versions/v0.6.4.tmp-unpack.<pid>` — never collides with a sibling version
/// or a concurrent install of the same one.
fn unpack_tmp_dir(dest: &Path) -> std::path::PathBuf {
    let name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("install");
    let tmp_name = format!(".{name}.tmp-unpack.{}", std::process::id());
    match dest.parent() {
        Some(parent) => parent.join(tmp_name),
        None => std::path::PathBuf::from(tmp_name),
    }
}

/// Extract a `.tar.gz` flattening the single top-level staging directory
/// (`ao-<version>-<target>/`) so files land directly under `dest`.
fn unpack_flatten(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = Archive::new(GzDecoder::new(bytes));
    let entries = archive.entries().map_err(|e| format!("read tar: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("read tar entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("tar entry path: {e}"))?
            .into_owned();
        // Reject symlink/hardlink entries outright: a link created inside `dest` could
        // otherwise let a later entry's write follow it outside `dest`. ao-cli archives
        // contain only regular files + dirs, so this loses nothing legitimate.
        let etype = entry.header().entry_type();
        if etype.is_symlink() || etype.is_hard_link() {
            return Err(format!(
                "refusing link entry in archive: {}",
                path.display()
            ));
        }
        let stripped = match safe_flatten(&path) {
            FlattenResult::Skip => continue,
            FlattenResult::Reject => {
                return Err(format!("tar entry escapes destination: {}", path.display()))
            }
            FlattenResult::Path(p) => p,
        };
        let out = dest.join(&stripped);
        entry
            .unpack(&out)
            .map_err(|e| format!("unpack {}: {e}", stripped.display()))?;
    }
    Ok(())
}

enum FlattenResult {
    Skip,
    Reject,
    Path(std::path::PathBuf),
}

/// Strip the single top-level staging directory from a tar entry path and reject
/// anything that isn't a sequence of `Normal` components — so the result, joined to
/// the destination, can never escape it (no `..`, no absolute roots, no prefixes).
fn safe_flatten(path: &Path) -> FlattenResult {
    use std::path::Component;
    // Drop leading `.` components (some tars emit `./staging/...`) before stripping the
    // single top-level staging directory, so a `./`-prefixed archive flattens correctly.
    let mut comps = path.components().peekable();
    while let Some(Component::CurDir) = comps.peek() {
        comps.next();
    }
    let stripped: std::path::PathBuf = comps.skip(1).collect();
    if stripped.as_os_str().is_empty() {
        return FlattenResult::Skip;
    }
    if stripped
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return FlattenResult::Reject;
    }
    FlattenResult::Path(stripped)
}

/// SHA256SUMS.txt lines are `<hex>  <basename>`.
fn expected_hash(sums: &str, name: &str) -> Option<String> {
    for line in sums.lines() {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        let file = it.next()?;
        let file = file.trim_start_matches('*');
        if file == name {
            return Some(hash.to_string());
        }
    }
    None
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "avm")
        .call()
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

fn http_get_string(url: &str) -> Result<String, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "avm")
        .call()
        .map_err(|e| e.to_string())?;
    resp.into_string().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sha256sums_line() {
        let sums = "abc123  ao-v0.6.4-x86_64-apple-darwin.tar.gz\ndef456  other.tar.gz\n";
        assert_eq!(
            expected_hash(sums, "ao-v0.6.4-x86_64-apple-darwin.tar.gz"),
            Some("abc123".into())
        );
        assert_eq!(expected_hash(sums, "missing.tar.gz"), None);
    }

    #[test]
    fn flatten_strips_top_level_and_keeps_normal() {
        match safe_flatten(Path::new("ao-v0.6.4-x86_64-apple-darwin/animus")) {
            FlattenResult::Path(p) => assert_eq!(p, Path::new("animus")),
            _ => panic!("expected a flattened path"),
        }
    }

    #[test]
    fn flatten_handles_curdir_prefix() {
        match safe_flatten(Path::new("./ao-v0.6.4-x86_64-apple-darwin/animus")) {
            FlattenResult::Path(p) => assert_eq!(p, Path::new("animus")),
            _ => panic!("expected a flattened path for a ./-prefixed entry"),
        }
    }

    #[test]
    fn flatten_rejects_parent_dir_traversal() {
        assert!(matches!(
            safe_flatten(Path::new("ao-vX/../escape")),
            FlattenResult::Reject
        ));
        assert!(matches!(
            safe_flatten(Path::new("ao-vX/sub/../../escape")),
            FlattenResult::Reject
        ));
    }

    #[test]
    fn tmp_dir_is_version_specific() {
        let a = unpack_tmp_dir(Path::new("/v/versions/v0.6.4"));
        let b = unpack_tmp_dir(Path::new("/v/versions/v0.6.5"));
        assert_ne!(a, b);
        assert!(a.to_str().unwrap().contains("v0.6.4"));
        assert!(b.to_str().unwrap().contains("v0.6.5"));
        assert_eq!(a.parent(), Some(Path::new("/v/versions")));
    }

    #[test]
    fn flatten_skips_bare_top_level_dir() {
        assert!(matches!(
            safe_flatten(Path::new("ao-vX")),
            FlattenResult::Skip
        ));
    }

    #[test]
    fn parses_starred_binary_marker() {
        let sums = "deadbeef *ao-v0.1.0-aarch64-apple-darwin.tar.gz\n";
        assert_eq!(
            expected_hash(sums, "ao-v0.1.0-aarch64-apple-darwin.tar.gz"),
            Some("deadbeef".into())
        );
    }
}
