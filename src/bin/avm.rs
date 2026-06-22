use avm::remote::list_remote;
use avm::{
    animus_exe_name, global_version_file, normalize_checked, resolve, shims_dir,
    version_binary_path, version_install_dir, versions_dir, ResolveSource, VERSION_FILE,
};

fn checked(raw: &str) -> Result<String, String> {
    normalize_checked(raw).ok_or_else(|| {
        format!(
            "invalid version `{raw}` — expected a release tag like `v0.6.4` (no path separators)"
        )
    })
}

const USAGE: &str = "\
avm — Animus Version Manager

USAGE:
    avm <command> [args]

COMMANDS:
    install [<version>]      Download + verify + unpack a version into ~/.avm/versions/<version>/
                             With no arg, installs the version resolved for the cwd.
    use <version>            Pin <version> for this project (writes ./.animus-version)
    use --global <version>   Set the global default (writes ~/.avm/version)
    local <version>          Alias for project pin
    list                     List installed versions
    list --remote            List available release tags from GitHub
    current                  Show the version that WOULD run here, its source, and path
    which                    Alias for current
    uninstall <version>      Remove an installed version
    shim-dir                 Print the shims directory to add to PATH
    help                     Show this help

The shim resolves versions in this precedence:
    1. ANIMUS_BIN (absolute path) / AVM_ANIMUS_VERSION env
    2. --project-root <path>/.animus-version
    3. nearest .animus-version walking up from cwd
    4. global default ~/.avm/version
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = run(&args);
    std::process::exit(code);
}

fn run(args: &[String]) -> i32 {
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let result = match cmd {
        "install" => cmd_install(rest),
        "use" => cmd_use(rest),
        "local" => cmd_local(rest),
        "list" | "ls" => cmd_list(rest),
        "current" | "which" => cmd_current(),
        "uninstall" | "remove" | "rm" => cmd_uninstall(rest),
        "shim-dir" => {
            println!("{}", shims_dir().display());
            Ok(())
        }
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("avm: {e}");
            1
        }
    }
}

fn cmd_install(rest: &[String]) -> Result<(), String> {
    let version = match rest.first() {
        Some(v) => checked(v)?,
        None => resolve_cwd_version()
            .ok_or("no version given and none resolved for the current directory")?,
    };
    avm::install::install(&version)
}

fn cmd_use(rest: &[String]) -> Result<(), String> {
    let global = rest.iter().any(|a| a == "--global" || a == "-g");
    let raw = rest
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("usage: avm use [--global] <version>")?;
    let version = checked(raw)?;

    if global {
        let gf = global_version_file();
        if let Some(parent) = gf.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&gf, format!("{version}\n")).map_err(|e| e.to_string())?;
        warn_if_missing(&version);
        println!("set global default -> {version} ({})", gf.display());
    } else {
        write_project_pin(&version)?;
    }
    Ok(())
}

fn cmd_local(rest: &[String]) -> Result<(), String> {
    let raw = rest.first().ok_or("usage: avm local <version>")?;
    let version = checked(raw)?;
    write_project_pin(&version)
}

fn write_project_pin(version: &str) -> Result<(), String> {
    let path = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join(VERSION_FILE);
    std::fs::write(&path, format!("{version}\n")).map_err(|e| e.to_string())?;
    warn_if_missing(version);
    println!("pinned {version} -> {}", path.display());
    Ok(())
}

fn warn_if_missing(version: &str) {
    if !version_binary_path(version).is_file() {
        eprintln!("avm: note: {version} is not installed yet — run `avm install {version}`");
    }
}

fn cmd_list(rest: &[String]) -> Result<(), String> {
    if rest.iter().any(|a| a == "--remote" || a == "-r") {
        let tags = list_remote()?;
        if tags.is_empty() {
            println!("(no releases found)");
        }
        for t in tags {
            println!("{t}");
        }
        return Ok(());
    }

    let dir = versions_dir();
    let active = resolve_cwd_version();
    let mut found = false;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().join(animus_exe_name()).is_file())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        for name in names {
            found = true;
            let marker = if Some(&name) == active.as_ref() {
                "* "
            } else {
                "  "
            };
            println!("{marker}{name}");
        }
    }
    if !found {
        println!("(no versions installed — run `avm install <version>`)");
    }
    Ok(())
}

fn cmd_current() -> Result<(), String> {
    let argv: Vec<String> = Vec::new();
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let res = resolve(&argv, &cwd, &|k| std::env::var(k).ok(), &|p| {
        std::fs::read_to_string(p).ok()
    });

    let source = match &res.source {
        ResolveSource::EnvPin => "AVM_ANIMUS_VERSION env".to_string(),
        ResolveSource::BinOverride => "ANIMUS_BIN override".to_string(),
        ResolveSource::ProjectRootFlag(p) => format!("--project-root ({})", p.display()),
        ResolveSource::CwdWalk(p) => format!(".animus-version ({})", p.display()),
        ResolveSource::Global => format!("global default ({})", global_version_file().display()),
    };

    if let Some(bin) = res.bin_override {
        println!("override: {} [source: {source}]", bin.display());
        return Ok(());
    }
    match res.version {
        Some(v) => {
            let bin = version_binary_path(&v);
            let installed = if bin.is_file() {
                "installed"
            } else {
                "NOT installed"
            };
            println!("{v} [source: {source}]");
            println!("path: {} ({installed})", bin.display());
            if !bin.is_file() {
                return Err(format!("{v} is not installed — run `avm install {v}`"));
            }
            Ok(())
        }
        None => Err(format!(
            "no version resolved [source: {source}] — pin one with `avm use <version>`"
        )),
    }
}

fn cmd_uninstall(rest: &[String]) -> Result<(), String> {
    let raw = rest.first().ok_or("usage: avm uninstall <version>")?;
    let version = checked(raw)?;
    let dir = version_install_dir(&version);
    if !dir.exists() {
        return Err(format!("{version} is not installed"));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    println!("uninstalled {version}");
    Ok(())
}

fn resolve_cwd_version() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let res = resolve(&[], &cwd, &|k| std::env::var(k).ok(), &|p| {
        std::fs::read_to_string(p).ok()
    });
    res.version
}
