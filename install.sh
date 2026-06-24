#!/usr/bin/env sh
# avm installer — downloads the avm manager + the `animus` shim and puts the
# shim first on your PATH, so `animus` in any project uses that project's pinned
# kernel version. Re-runnable (upgrades in place).
#
#   curl -fsSL https://raw.githubusercontent.com/launchapp-dev/avm/main/install.sh | sh
#
# Honors:
#   AVM_VERSION   release tag to install (default: latest)
#   AVM_HOME      install root (default: ~/.avm)
#   AVM_NO_PROFILE=1   skip editing the shell profile (just print the PATH line)
set -eu

REPO="launchapp-dev/avm"
AVM_HOME="${AVM_HOME:-$HOME/.avm}"
BIN_DIR="$AVM_HOME/bin"
SHIM_DIR="$AVM_HOME/shims"

say() { printf '%s\n' "$*"; }
err() { printf 'avm-install: %s\n' "$*" >&2; exit 1; }

# --- detect target triple -------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) plat="apple-darwin" ;;
  Linux)  plat="unknown-linux-gnu" ;;
  *) err "unsupported OS: $os" ;;
esac
case "$arch" in
  x86_64|amd64) cpu="x86_64" ;;
  arm64|aarch64) cpu="aarch64" ;;
  *) err "unsupported arch: $arch" ;;
esac
target="${cpu}-${plat}"

# --- resolve version ------------------------------------------------------
version="${AVM_VERSION:-}"
if [ -z "$version" ]; then
  version="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$version" ] || err "could not resolve the latest avm release; set AVM_VERSION=vX.Y.Z"
fi

archive="avm-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${version}/${archive}"
say "Installing avm ${version} (${target})…"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fsSL -o "$tmp/$archive" "$url" || err "download failed: $url"
# Verify checksum when the sidecar is present.
if curl -fsSL -o "$tmp/$archive.sha256" "${url}.sha256" 2>/dev/null; then
  ( cd "$tmp" && sed "s#\$# *${archive}#" "$archive.sha256" >/dev/null 2>&1 || true )
  if command -v shasum >/dev/null 2>&1; then
    want="$(awk '{print $1}' "$tmp/$archive.sha256")"
    got="$(shasum -a 256 "$tmp/$archive" | awk '{print $1}')"
    [ "$want" = "$got" ] || err "checksum mismatch for $archive (want $want, got $got)"
  fi
fi

mkdir -p "$BIN_DIR" "$SHIM_DIR"
tar -xzf "$tmp/$archive" -C "$tmp"
install -m 0755 "$tmp/avm" "$BIN_DIR/avm"
# The `animus` shim lives in the shims dir (first on PATH); `avm` lives in bin.
install -m 0755 "$tmp/animus" "$SHIM_DIR/animus"
say "Installed: $BIN_DIR/avm  +  $SHIM_DIR/animus"

# --- PATH wiring ----------------------------------------------------------
path_line='export PATH="$HOME/.avm/shims:$HOME/.avm/bin:$PATH"'
profile=""
case "${SHELL:-}" in
  */zsh) profile="$HOME/.zshrc" ;;
  */bash) [ -f "$HOME/.bashrc" ] && profile="$HOME/.bashrc" || profile="$HOME/.bash_profile" ;;
  *) profile="$HOME/.profile" ;;
esac
if [ "${AVM_NO_PROFILE:-0}" != "1" ] && [ -n "$profile" ]; then
  if [ ! -f "$profile" ] || ! grep -q '.avm/shims' "$profile" 2>/dev/null; then
    printf '\n# avm — Animus Version Manager (shim first on PATH)\n%s\n' "$path_line" >> "$profile"
    say "Added avm to PATH in $profile"
  fi
fi

say ""
say "avm ${version} installed. Open a new shell (or run: ${path_line#export }) then:"
say "  avm install <version>        # e.g. avm install v0.6.9"
say "  avm use --global <version>   # machine default"
say "  avm use <version>            # pin THIS project (writes ./.animus-version)"
