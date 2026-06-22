use avm::{
    normalize_version, resolve, version_binary_path, Resolution, ResolveSource, ENV_AUTO_INSTALL,
    ENV_PIN,
};
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    let res = resolve(&argv, &cwd, &|k| std::env::var(k).ok(), &|p| {
        std::fs::read_to_string(p).ok()
    });

    match &res.source {
        ResolveSource::BinOverride => {
            let bin = res.bin_override.clone().unwrap();
            exec_animus(&bin, &argv, None);
        }
        _ => dispatch_versioned(res, &argv),
    }
}

fn dispatch_versioned(res: Resolution, argv: &[String]) -> ! {
    let version = match res.version {
        Some(v) => v,
        None => {
            eprintln!(
                "avm: no animus version resolved.\n\
                 Pin one with `avm use <version>` (project) or `avm use --global <version>`,\n\
                 or set {ENV_PIN}=<version>."
            );
            std::process::exit(1);
        }
    };

    let mut bin = version_binary_path(&version);
    if !bin.is_file() {
        let auto = std::env::var(ENV_AUTO_INSTALL)
            .map(|v| v == "1")
            .unwrap_or(false);
        if auto {
            eprintln!("avm: {version} not installed; auto-installing ({ENV_AUTO_INSTALL}=1)");
            if let Err(e) = avm::install::install(&version) {
                eprintln!("avm: auto-install of {version} failed: {e}");
                std::process::exit(1);
            }
            bin = version_binary_path(&version);
        } else {
            eprintln!(
                "avm: animus {version} is not installed.\n\
                 Run: avm install {version}\n\
                 (or set {ENV_AUTO_INSTALL}=1 to install on demand)"
            );
            std::process::exit(1);
        }
    }

    // Pin the resolved version into the child env so any recursive `animus`
    // spawn (mcp serve, the daemon re-spawning itself from a different cwd)
    // resolves to the SAME version instead of re-walking from elsewhere.
    exec_animus(&bin, argv, Some(&version));
}

#[cfg(unix)]
fn exec_animus(bin: &std::path::Path, argv: &[String], pin: Option<&str>) -> ! {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(bin);
    cmd.args(argv);
    if let Some(v) = pin {
        cmd.env(ENV_PIN, normalize_version(v));
    }
    let err = cmd.exec();
    eprintln!("avm: failed to exec {}: {err}", bin.display());
    std::process::exit(127);
}

#[cfg(not(unix))]
fn exec_animus(bin: &std::path::Path, argv: &[String], pin: Option<&str>) -> ! {
    // No process-image replacement on Windows; spawn + forward the exit code.
    let mut cmd = Command::new(bin);
    cmd.args(argv);
    if let Some(v) = pin {
        cmd.env(ENV_PIN, normalize_version(v));
    }
    match cmd.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("avm: failed to run {}: {e}", bin.display());
            std::process::exit(127);
        }
    }
}
