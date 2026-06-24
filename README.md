# avm — Animus Version Manager

`avm` pins each project to a specific [`animus`](https://github.com/launchapp-dev/animus-cli)
kernel version and dispatches to it transparently, the way `rustup`, `nvm`, or `asdf`
do for their toolchains. A project declares the version it needs in a `.animus-version`
file; running `animus` in that project then automatically uses the right kernel, with
no explicit switch step.

avm is **kernel-version-agnostic** — it manages many `animus` versions and is a separate
tool from the kernel itself. It consumes the published `animus-cli` release tarballs.

## How it works

Two binaries ship from this repo:

- **`avm`** — the manager (install / use / list / current / uninstall).
- **`animus`** — a tiny shim placed on your `PATH`. On every invocation it resolves the
  target version, then `exec`s the real `animus` binary for that version with your argv
  unchanged. Exit codes, signals, and stdio pass through verbatim.

## Install

**One-liner (recommended).** Downloads the `avm` manager + the `animus` shim,
puts the shim first on your `PATH`, and is safe to re-run (upgrades in place):

```sh
curl -fsSL https://raw.githubusercontent.com/launchapp-dev/avm/main/install.sh | sh
```

Then open a new shell and install a kernel:

```sh
avm install v0.6.9            # download a kernel version
avm use --global v0.6.9       # machine default
animus --version             # now resolved through avm
```

The installer drops `avm` into `~/.avm/bin`, the `animus` shim into `~/.avm/shims`,
and adds `export PATH="$HOME/.avm/shims:$HOME/.avm/bin:$PATH"` to your shell profile
(ahead of any other `animus`). Env knobs: `AVM_VERSION` (pin the avm release),
`AVM_HOME` (install root), `AVM_NO_PROFILE=1` (don't edit the profile).

**From source (Cargo).** Builds both binaries into `~/.cargo/bin`:

```sh
cargo install --git https://github.com/launchapp-dev/avm   # or: cargo install --path .
mkdir -p ~/.avm/shims
# link the Cargo-installed shim explicitly — do NOT use `command -v animus`,
# which would resolve to the shim itself once ~/.avm/shims is on PATH.
ln -sf "${CARGO_HOME:-$HOME/.cargo}/bin/animus" ~/.avm/shims/animus
export PATH="$HOME/.avm/shims:$PATH"   # add to your shell profile, before other animus
```

`avm shim-dir` prints the directory to add to `PATH`.

## Usage

```sh
avm install v0.6.4            # download + verify + unpack into ~/.avm/versions/v0.6.4/
avm install                  # install whatever version the cwd resolves to

avm use v0.6.4               # pin this project -> writes ./.animus-version
avm local v0.6.4             # alias for the project pin
avm use --global v0.6.4      # set the global default -> ~/.avm/version

avm list                     # installed versions (active one marked with *)
avm list --remote            # available release tags from GitHub
avm current                  # the version that WOULD run here + its source + path
avm which                    # alias for current
avm uninstall v0.5.0         # remove an installed version
```

Set `AVM_AUTO_INSTALL=1` to have the shim install a missing resolved version on demand
instead of erroring.

## Resolution precedence

On every `animus` invocation, the shim resolves the version in this strict order:

1. **`ANIMUS_BIN`** — if set to an absolute path, that binary is used directly
   (escape hatch); otherwise **`AVM_ANIMUS_VERSION`** env var.
2. **`--project-root <path>`** present in argv → read `<path>/.animus-version`.
3. Nearest **`.animus-version`** found walking **up** from the current directory.
4. Global default in **`~/.avm/version`**.

If no version resolves, or the resolved version is not installed, the shim exits with an
actionable error telling you the exact `avm install <version>` command to run.

### Consistent version for a process and all its children

`animus` spawns other `animus` processes (e.g. `animus mcp serve` over stdio, and the
daemon re-spawning itself), often from a *different* working directory. To guarantee a
parent and all its descendants run the **same** kernel version, once the shim resolves a
version it exports `AVM_ANIMUS_VERSION=<resolved>` into the child's environment. Because
that env var is the highest-precedence file-independent source, every recursive spawn
short-circuits straight to the already-resolved version instead of re-walking from
wherever it happens to be.

## `.animus-version` format

A single line containing the version string, with or without a leading `v`. Blank lines
and `#` comments are ignored.

```
v0.6.4
```

`0.6.4` and `v0.6.4` are equivalent; both normalize to `v0.6.4`.

## Releases and verification

`avm install <version>` downloads
`ao-<version>-<target>.tar.gz` from
`https://github.com/launchapp-dev/animus-cli/releases/download/<version>/`, where
`<target>` is the host's Rust target triple (e.g. `aarch64-apple-darwin`,
`x86_64-unknown-linux-gnu`). It then downloads `SHA256SUMS.txt` published alongside the
tarballs and verifies the SHA-256 of the downloaded archive against the listed hash
before unpacking. A mismatch aborts the install. The archive's single top-level staging
directory is flattened so the `animus` binary lands directly in
`~/.avm/versions/<version>/animus`.

`avm list --remote` queries the GitHub releases API for available tags (honoring
`GITHUB_TOKEN` for rate limits).

## State layout

- `~/.avm/versions/<version>/animus` — installed kernels
- `~/.avm/version` — global default version
- `~/.avm/shims/animus` — the shim on PATH
- `./.animus-version` — per-project pin

Override the root with `AVM_HOME`.

## Platform note

The shim uses `exec` (process-image replacement) on Unix so exit codes and signals pass
through unchanged. On Windows it spawns the child and forwards the exit code (true `exec`
semantics are not available there).
