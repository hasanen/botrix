#!/bin/sh
# Install Trunk into the devcontainer (Linux) from its pinned release binary.
# The version and checksums are read from the Containerfile, so there is a
# single place to bump them. Does nothing if that version is already installed.
set -eu

cd "$(dirname "$0")/.."

arg() { sed -n "s/^ARG $1=//p" Containerfile; }

version=$(arg TRUNK_VERSION)
case "$(uname -m)" in
  x86_64)        arch=x86_64;  sha=$(arg TRUNK_SHA256_AMD64) ;;
  aarch64|arm64) arch=aarch64; sha=$(arg TRUNK_SHA256_ARM64) ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
if "$bin_dir/trunk" --version 2>/dev/null | grep -q " $version\$"; then
  exit 0
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
curl --proto '=https' --tlsv1.2 -fsSL -o "$tmp" \
  "https://github.com/trunk-rs/trunk/releases/download/v${version}/trunk-${arch}-unknown-linux-musl.tar.gz"
echo "${sha}  ${tmp}" | sha256sum -c -
mkdir -p "$bin_dir"
tar -xzf "$tmp" -C "$bin_dir" trunk
"$bin_dir/trunk" --version
