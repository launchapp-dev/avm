use std::path::{Path, PathBuf};

pub mod install;
pub mod remote;

pub const RELEASE_REPO: &str = "launchapp-dev/animus-cli";
pub const VERSION_FILE: &str = ".animus-version";
pub const ENV_PIN: &str = "AVM_ANIMUS_VERSION";
pub const ENV_BIN_OVERRIDE: &str = "ANIMUS_BIN";
pub const ENV_AUTO_INSTALL: &str = "AVM_AUTO_INSTALL";

/// Normalize a version string to a canonical leading-`v` form.
/// Accepts `0.6.4` or `v0.6.4`, both resolve to `v0.6.4`.
///
/// This performs string shaping only and does NOT validate the charset — callers that
/// turn a version into a filesystem path MUST go through [`normalize_checked`] (or
/// validate with [`is_valid_version`]) so untrusted `.animus-version` values can never
/// contain path separators or `..` and escape the versions directory.
pub fn normalize_version(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('v') || trimmed.starts_with('V') {
        format!("v{}", trimmed[1..].trim_start_matches('V').trim())
    } else {
        format!("v{}", trimmed)
    }
}

/// A version is valid only if, after normalization, its body (sans the leading `v`) is a
/// non-empty run of ASCII alphanumerics, `.`, `-`, or `_` — the charset GitHub release
/// tags use. This rejects `/`, `\`, `..`, whitespace, and other path-significant input.
pub fn is_valid_version(version: &str) -> bool {
    let body = version.strip_prefix('v').unwrap_or(version);
    !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !body.contains("..")
}

/// Normalize and validate a version string. Returns `None` if the result would not be a
/// safe, path-free version token.
pub fn normalize_checked(raw: &str) -> Option<String> {
    let v = normalize_version(raw);
    if is_valid_version(&v) {
        Some(v)
    } else {
        None
    }
}

/// Parse the first non-empty, non-comment line of a `.animus-version` file body.
/// Returns `None` for blank/comment-only bodies and for values that fail validation.
pub fn parse_version_file(contents: &str) -> Option<String> {
    contents
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .and_then(normalize_checked)
}

/// Root of avm state, honoring `AVM_HOME`, else `~/.avm`.
pub fn avm_home() -> PathBuf {
    if let Some(h) = std::env::var_os("AVM_HOME") {
        return PathBuf::from(h);
    }
    home_dir().join(".avm")
}

pub fn versions_dir() -> PathBuf {
    avm_home().join("versions")
}

/// Build the install directory for a version. The version is validated and any
/// path-significant input is collapsed to a single inert component, so the returned path
/// is always a direct child of [`versions_dir`] and can never traverse outside it.
pub fn version_install_dir(version: &str) -> PathBuf {
    let safe = normalize_checked(version).unwrap_or_else(|| "__invalid__".to_string());
    versions_dir().join(safe)
}

/// The kernel executable file name for the host platform (`animus`, or `animus.exe` on Windows).
pub fn animus_exe_name() -> &'static str {
    if cfg!(windows) {
        "animus.exe"
    } else {
        "animus"
    }
}

pub fn version_binary_path(version: &str) -> PathBuf {
    version_install_dir(version).join(animus_exe_name())
}

pub fn global_version_file() -> PathBuf {
    avm_home().join("version")
}

pub fn shims_dir() -> PathBuf {
    avm_home().join("shims")
}

fn home_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h);
    }
    if let Some(h) = std::env::var_os("USERPROFILE") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveSource {
    EnvPin,
    BinOverride,
    ProjectRootFlag(PathBuf),
    CwdWalk(PathBuf),
    Global,
}

#[derive(Debug, Clone)]
pub struct Resolution {
    pub version: Option<String>,
    pub bin_override: Option<PathBuf>,
    pub source: ResolveSource,
}

/// Resolve the animus version for an invocation, in strict precedence order:
///   1. ANIMUS_BIN (absolute path override)  -> BinOverride
///   2. AVM_ANIMUS_VERSION env               -> EnvPin
///   3. --project-root <path> in argv        -> read <path>/.animus-version
///   4. walk UP from cwd for .animus-version
///   5. global default ~/.avm/version
///
/// `env_lookup` and file readers are injected for testability.
pub fn resolve(
    argv: &[String],
    cwd: &Path,
    env: &dyn Fn(&str) -> Option<String>,
    read_file: &dyn Fn(&Path) -> Option<String>,
) -> Resolution {
    if let Some(bin) = env(ENV_BIN_OVERRIDE) {
        let p = PathBuf::from(bin);
        if p.is_absolute() {
            return Resolution {
                version: None,
                bin_override: Some(p),
                source: ResolveSource::BinOverride,
            };
        }
    }

    if let Some(raw) = env(ENV_PIN).filter(|v| !v.trim().is_empty()) {
        // An invalid env pin is surfaced as a resolution with no version (the shim then
        // errors actionably) rather than silently falling through to a file/global value.
        return Resolution {
            version: normalize_checked(&raw),
            bin_override: None,
            source: ResolveSource::EnvPin,
        };
    }

    if let Some(root) = project_root_flag(argv) {
        let vf = root.join(VERSION_FILE);
        if let Some(v) = read_file(&vf).as_deref().and_then(parse_version_file) {
            return Resolution {
                version: Some(v),
                bin_override: None,
                source: ResolveSource::ProjectRootFlag(vf),
            };
        }
    }

    let mut dir: Option<&Path> = Some(cwd);
    while let Some(d) = dir {
        let vf = d.join(VERSION_FILE);
        if let Some(v) = read_file(&vf).as_deref().and_then(parse_version_file) {
            return Resolution {
                version: Some(v),
                bin_override: None,
                source: ResolveSource::CwdWalk(vf),
            };
        }
        dir = d.parent();
    }

    let gf = global_version_file();
    let v = read_file(&gf).as_deref().and_then(parse_version_file);
    Resolution {
        version: v,
        bin_override: None,
        source: ResolveSource::Global,
    }
}

/// Extract the value of `--project-root <path>` or `--project-root=<path>` from argv.
fn project_root_flag(argv: &[String]) -> Option<PathBuf> {
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if let Some(rest) = a.strip_prefix("--project-root=") {
            return Some(PathBuf::from(rest));
        }
        if a == "--project-root" {
            if let Some(val) = argv.get(i + 1) {
                return Some(PathBuf::from(val));
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from<'a>(map: &'a HashMap<&'a str, &'a str>) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| map.get(k).map(|v| v.to_string())
    }

    #[test]
    fn normalize_accepts_with_and_without_v() {
        assert_eq!(normalize_version("0.6.4"), "v0.6.4");
        assert_eq!(normalize_version("v0.6.4"), "v0.6.4");
        assert_eq!(normalize_version("  0.6.4  "), "v0.6.4");
        assert_eq!(normalize_version("V0.6.4"), "v0.6.4");
    }

    #[test]
    fn rejects_path_traversal_versions() {
        assert!(!is_valid_version("v0.6.4/../../etc"));
        assert!(!is_valid_version("v../../payload"));
        assert!(!is_valid_version("v0.6/4"));
        assert!(!is_valid_version("v0.6\\4"));
        assert!(!is_valid_version("v"));
        assert!(!is_valid_version("v ./x"));
        assert!(is_valid_version("v0.6.4"));
        assert!(is_valid_version("v0.6.4-rc1"));
        assert!(is_valid_version("v0.6.4_beta"));

        assert_eq!(normalize_checked("0.6.4"), Some("v0.6.4".into()));
        assert_eq!(normalize_checked("0.6.4/../../x"), None);
        assert_eq!(parse_version_file("0.6.4/../../x"), None);
    }

    #[test]
    fn version_install_dir_never_escapes() {
        let dir = version_install_dir("0.6.4/../../../../etc");
        assert_eq!(dir.parent(), Some(versions_dir().as_path()));
    }

    #[test]
    fn invalid_env_pin_yields_no_version() {
        let mut m = HashMap::new();
        m.insert(ENV_PIN, "0.6.4/../../x");
        let r = resolve(&[], Path::new("/x"), &env_from(&m), &|_| {
            Some("v1.0.0".into())
        });
        assert_eq!(r.source, ResolveSource::EnvPin);
        assert!(r.version.is_none());
    }

    #[test]
    fn parse_file_skips_blank_and_comments() {
        assert_eq!(
            parse_version_file("\n# comment\nv0.6.4\n"),
            Some("v0.6.4".into())
        );
        assert_eq!(parse_version_file("0.6.4"), Some("v0.6.4".into()));
        assert_eq!(parse_version_file("\n\n"), None);
        assert_eq!(parse_version_file("# only a comment"), None);
    }

    #[test]
    fn precedence_bin_override_wins() {
        let mut m = HashMap::new();
        m.insert(ENV_BIN_OVERRIDE, "/opt/animus/animus");
        m.insert(ENV_PIN, "v9.9.9");
        let r = resolve(&[], Path::new("/x"), &env_from(&m), &|_| {
            Some("v1.0.0".into())
        });
        assert_eq!(r.source, ResolveSource::BinOverride);
        assert_eq!(r.bin_override, Some(PathBuf::from("/opt/animus/animus")));
        assert!(r.version.is_none());
    }

    #[test]
    fn relative_bin_override_ignored() {
        let mut m = HashMap::new();
        m.insert(ENV_BIN_OVERRIDE, "relative/animus");
        m.insert(ENV_PIN, "v9.9.9");
        let r = resolve(&[], Path::new("/x"), &env_from(&m), &|_| None);
        assert_eq!(r.source, ResolveSource::EnvPin);
        assert_eq!(r.version, Some("v9.9.9".into()));
    }

    #[test]
    fn precedence_env_pin_over_flag_and_walk() {
        let mut m = HashMap::new();
        m.insert(ENV_PIN, "0.6.4");
        let argv = vec!["--project-root".into(), "/proj".into()];
        let r = resolve(&argv, Path::new("/proj/sub"), &env_from(&m), &|_| {
            Some("v1.0.0".into())
        });
        assert_eq!(r.source, ResolveSource::EnvPin);
        assert_eq!(r.version, Some("v0.6.4".into()));
    }

    #[test]
    fn precedence_project_root_flag_over_walk() {
        let m = HashMap::new();
        let argv = vec!["run".into(), "--project-root=/proj".into()];
        let read = |p: &Path| -> Option<String> {
            if p == Path::new("/proj/.animus-version") {
                Some("v0.5.0".into())
            } else {
                Some("v1.0.0".into())
            }
        };
        let r = resolve(&argv, Path::new("/elsewhere"), &env_from(&m), &read);
        assert!(matches!(r.source, ResolveSource::ProjectRootFlag(_)));
        assert_eq!(r.version, Some("v0.5.0".into()));
    }

    #[test]
    fn flag_falls_through_when_no_file_there() {
        let m = HashMap::new();
        let argv = vec!["--project-root".into(), "/proj".into()];
        let read = |p: &Path| -> Option<String> {
            if p == Path::new("/cwd/.animus-version") {
                Some("v0.3.0".into())
            } else {
                None
            }
        };
        let r = resolve(&argv, Path::new("/cwd"), &env_from(&m), &read);
        assert!(matches!(r.source, ResolveSource::CwdWalk(_)));
        assert_eq!(r.version, Some("v0.3.0".into()));
    }

    #[test]
    fn cwd_walk_finds_nearest_ancestor() {
        let m = HashMap::new();
        let read = |p: &Path| -> Option<String> {
            if p == Path::new("/a/b/.animus-version") {
                Some("0.2.0".into())
            } else {
                None
            }
        };
        let r = resolve(&[], Path::new("/a/b/c/d"), &env_from(&m), &read);
        assert!(matches!(r.source, ResolveSource::CwdWalk(_)));
        assert_eq!(r.version, Some("v0.2.0".into()));
    }

    #[test]
    fn cwd_walk_prefers_deepest() {
        let m = HashMap::new();
        let read = |p: &Path| -> Option<String> {
            match p.to_str().unwrap() {
                "/a/b/c/.animus-version" => Some("v3.0.0".into()),
                "/a/.animus-version" => Some("v1.0.0".into()),
                _ => None,
            }
        };
        let r = resolve(&[], Path::new("/a/b/c"), &env_from(&m), &read);
        assert_eq!(r.version, Some("v3.0.0".into()));
    }

    #[test]
    fn falls_back_to_global() {
        let m = HashMap::new();
        let gf = global_version_file();
        let gf2 = gf.clone();
        let read = move |p: &Path| -> Option<String> {
            if p == gf2 {
                Some("v0.1.0".into())
            } else {
                None
            }
        };
        let r = resolve(&[], Path::new("/no/version/here"), &env_from(&m), &read);
        assert_eq!(r.source, ResolveSource::Global);
        assert_eq!(r.version, Some("v0.1.0".into()));
    }

    #[test]
    fn global_missing_yields_none() {
        let m = HashMap::new();
        let r = resolve(&[], Path::new("/nope"), &env_from(&m), &|_| None);
        assert_eq!(r.source, ResolveSource::Global);
        assert!(r.version.is_none());
    }
}
