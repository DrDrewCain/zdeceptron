#!/usr/bin/env sh
#
# Install `zdc` from a GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/DrDrewCain/zdeceptron/main/scripts/install.sh | sh
#
# POSIX `sh` rather than bash: this is the one script a person runs before
# they have anything else, so it must work on a machine that has nothing
# else — including the Debian and Alpine images where `/bin/sh` is `dash`
# and `ash` and not bash wearing a hat.
#
# The checksum is verified. A pipe-to-shell installer that downloads a
# binary and does not check it against the hash published beside it is
# offering convenience in place of the guarantee it looks like it is
# making, and the check costs one command.

set -eu

REPO="DrDrewCain/zdeceptron"
BIN="zdc"
# Overridable so a user can put it somewhere on their PATH without sudo.
INSTALL_DIR="${ZDC_INSTALL_DIR:-/usr/local/bin}"
VERSION="${ZDC_VERSION:-latest}"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "this script needs \`$1\` and cannot find it"
}

need uname
need tar

# One of curl or wget, whichever is here.
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
    fetch_stdout() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
    fetch_stdout() { wget -qO- "$1"; }
else
    die "this script needs either \`curl\` or \`wget\` and cannot find either"
fi

# --- which build ------------------------------------------------------------

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
    Darwin) os_part="apple-darwin" ;;
    Linux)  os_part="unknown-linux-musl" ;;
    *)
        die "no prebuilt binary for \`$os\`. Build from source:
  git clone https://github.com/$REPO && cd zdeceptron && cargo build --release"
        ;;
esac

case "$arch" in
    x86_64 | amd64)  arch_part="x86_64" ;;
    arm64 | aarch64) arch_part="aarch64" ;;
    *)
        die "no prebuilt binary for \`$arch\`. Build from source:
  git clone https://github.com/$REPO && cd zdeceptron && cargo build --release"
        ;;
esac

target="${arch_part}-${os_part}"

# --- which version ----------------------------------------------------------

if [ "$VERSION" = "latest" ]; then
    say "Looking up the latest release…"
    # `tag_name` out of the API rather than following the /latest redirect:
    # the tag is needed to build the asset name, not just the URL.
    # `2>/dev/null` because the failure is handled below with a message
    # that says what to do, and curl's own `(22) 404` above it only makes
    # that harder to read.
    VERSION="$(
        fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' \
        | head -n 1
    )" || true
    [ -n "$VERSION" ] || die "could not find a published release.
The compiler can still be built from source:
  git clone https://github.com/$REPO && cd zdeceptron && cargo build --release"
fi

number="${VERSION#v}"
asset="zdc-${number}-${target}.tar.gz"
url="https://github.com/$REPO/releases/download/$VERSION/$asset"

say "Installing zdc $VERSION for $target…"

# --- download and verify ----------------------------------------------------

tmp="$(mktemp -d)"
# Cleans up on a failed download as well as a successful one.
trap 'rm -rf "$tmp"' EXIT INT TERM

fetch "$url" "$tmp/$asset" || die "could not download $url
Check that $VERSION has a build for $target."

if fetch "$url.sha256" "$tmp/$asset.sha256" 2>/dev/null; then
    say "Verifying checksum…"
    expected="$(cut -d' ' -f1 < "$tmp/$asset.sha256")"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$tmp/$asset" | cut -d' ' -f1)"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$tmp/$asset" | cut -d' ' -f1)"
    else
        actual=""
        say "warning: no sha256sum or shasum; skipping verification"
    fi
    if [ -n "$actual" ] && [ "$actual" != "$expected" ]; then
        die "checksum mismatch for $asset
  expected $expected
  got      $actual
Not installing. This means the download was corrupted or tampered with."
    fi
else
    # Loud rather than silent: "there was no checksum to check" and "the
    # checksum matched" must not look the same to whoever ran this.
    say "warning: no published checksum for $asset; installing unverified"
fi

tar xzf "$tmp/$asset" -C "$tmp"
extracted="$tmp/zdc-${number}-${target}/$BIN"
[ -f "$extracted" ] || die "the archive did not contain \`$BIN\`"
chmod +x "$extracted"

# --- install ----------------------------------------------------------------

if [ -w "$INSTALL_DIR" ]; then
    mv "$extracted" "$INSTALL_DIR/$BIN"
elif command -v sudo >/dev/null 2>&1; then
    say "$INSTALL_DIR is not writable; using sudo"
    sudo mv "$extracted" "$INSTALL_DIR/$BIN"
else
    die "$INSTALL_DIR is not writable and \`sudo\` is not available.
Set a writable directory and run again:
  ZDC_INSTALL_DIR=\"\$HOME/.local/bin\" sh install.sh"
fi

say ""
say "zdc $VERSION installed to $INSTALL_DIR/$BIN"

if command -v "$BIN" >/dev/null 2>&1; then
    say "$("$BIN" --version)"
    say ""
    say "Try it:"
    say "  zdc --help"
else
    say ""
    say "warning: $INSTALL_DIR is not on your PATH. Add it:"
    say "  export PATH=\"$INSTALL_DIR:\$PATH\""
fi
