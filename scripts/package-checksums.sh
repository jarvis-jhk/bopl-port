#!/bin/bash
# Writes dist/SHA256SUMS for every release archive in dist/ (the format
# `sha256sum -c` reads, and the file the Warpzone descriptor names under
# [install] checksums). Re-run after adding archives; the release workflow
# runs it once over the archives of both platforms.
# Usage: scripts/package-checksums.sh [DIR]   (default: dist/)
set -euo pipefail
source "$(dirname "$0")/../env.sh"
DIR="${1:-${BOPL_DIST_DIR:-$BOPL_ROOT/dist}}"
cd "$DIR"
shopt -s nullglob
files=(bopl-port-*-linux-x86_64.tar.gz bopl-port-*-windows-x86_64.zip)
[ ${#files[@]} -gt 0 ] || { echo "no release archives in $DIR" >&2; exit 1; }
sha256sum "${files[@]}" > SHA256SUMS
sha256sum -c --quiet SHA256SUMS
cat SHA256SUMS
