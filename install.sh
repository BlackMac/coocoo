#!/bin/sh
# Installs the `coocoo` daemon/CLI from a GitHub release, no Rust toolchain needed:
#   curl -fsSL https://coocoo.dev/install.sh | sh
# Environment:
#   COOCOO_VERSION      release tag, e.g. v0.5.0 (default: the latest release)
#   COOCOO_INSTALL_DIR  target directory (default: ~/.local/bin)
#   COOCOO_RELEASES_URL release base URL (default: https://github.com/BlackMac/coocoo/releases)
# The daemon also runs this script on a server when it installs coocoo over SSH.
set -eu

fail() {
    echo "coocoo install: $*" >&2
    exit 1
}

case "$(uname -s)" in
    Darwin) target=macos-universal ;;
    Linux)
        case "$(uname -m)" in
            x86_64 | amd64) target=linux-x86_64 ;;
            aarch64 | arm64) target=linux-aarch64 ;;
            *) fail "no prebuilt coocoo for Linux $(uname -m); build it from source (see README)" ;;
        esac
        ;;
    *) fail "no prebuilt coocoo for $(uname -s); build it from source (see README)" ;;
esac

releases="${COOCOO_RELEASES_URL:-https://github.com/BlackMac/coocoo/releases}"
asset="coocoo-$target.tar.gz"
if [ -n "${COOCOO_VERSION:-}" ]; then
    url="$releases/download/$COOCOO_VERSION/$asset"
else
    url="$releases/latest/download/$asset"
fi
dir="${COOCOO_INSTALL_DIR:-$HOME/.local/bin}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 2 -o "$tmp/$asset" "$url" || fail "download failed: $url"
elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$tmp/$asset" "$url" || fail "download failed: $url"
else
    fail "needs curl or wget"
fi
tar -xzf "$tmp/$asset" -C "$tmp" || fail "cannot unpack $asset"
[ -f "$tmp/coocoo" ] || fail "$asset contains no coocoo binary"

mkdir -p "$dir"
# Move into place: replacing the file of a running daemon must not touch its open inode.
cp "$tmp/coocoo" "$dir/coocoo.new"
chmod 755 "$dir/coocoo.new"
mv -f "$dir/coocoo.new" "$dir/coocoo"

echo "Installed $("$dir/coocoo" version) to $dir/coocoo"
case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "Add $dir to your PATH, e.g.: echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile" ;;
esac
