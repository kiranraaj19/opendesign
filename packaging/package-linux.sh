#!/usr/bin/env bash
# Minimal Linux packaging for OpenDesign: build a release binary and
# bundle it with the top-level docs into a .tar.gz under dist/ (tar+gzip
# rather than zip since both ship with every Linux install this targets;
# `zip` itself is often not installed).
#
# This is deliberately not a cargo-dist / installer setup — no signing,
# no system package (.deb/.rpm/AppImage), no auto-update. It's the
# smallest thing that gets a runnable build into one file a user can
# download and unpack. Phase 13's roadmap note on a "real" installer is
# future work; see packaging/README.md for what that would take,
# especially on Windows.
#
# Usage: packaging/package-linux.sh [target-triple]
# Defaults to the host triple (run `rustc -vV` to see what that is).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

target="${1:-}"
bin_name="opendesign"
mcp_name="opendesign-mcp"
version="$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)"/\1/')"

if [[ -n "$target" ]]; then
    cargo build --release --target "$target" -p od_app -p od_mcp
    bin_path="target/$target/release/$bin_name"
    archive_name="opendesign-${version}-${target}.tar.gz"
else
    cargo build --release -p od_app -p od_mcp
    bin_path="target/release/$bin_name"
    host="$(rustc -vV | sed -n 's/^host: //p')"
    archive_name="opendesign-${version}-${host}.tar.gz"
fi

if [[ ! -x "$bin_path" ]]; then
    echo "error: expected binary at $bin_path, build did not produce it" >&2
    exit 1
fi

stage_dir="$(mktemp -d)"
trap 'rm -rf "$stage_dir"' EXIT
pkg_dir="$stage_dir/opendesign-${version}"
mkdir -p "$pkg_dir"
cp "$bin_path" "$pkg_dir/"
# The MCP server, for driving OpenDesign from an AI assistant.
cp "$(dirname "$bin_path")/$mcp_name" "$pkg_dir/"
cp README.md "$pkg_dir/" 2>/dev/null || true
cp LICENSE "$pkg_dir/" 2>/dev/null || true
cp LICENSE-MIT "$pkg_dir/" 2>/dev/null || true
cp LICENSE-APACHE "$pkg_dir/" 2>/dev/null || true

mkdir -p dist
out="dist/$archive_name"
rm -f "$out"
tar -C "$stage_dir" -czf "$repo_root/$out" "$(basename "$pkg_dir")"

echo "Wrote $out"
