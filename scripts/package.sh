#!/bin/bash
# Builds both release archives in dist/ (nothing is published):
#   dist/bopl-port-<version>-linux-x86_64.tar.gz   (scripts/package-linux.sh)
#   dist/bopl-port-<version>-windows-x86_64.zip    (scripts/package-windows.sh)
#   dist/SHA256SUMS
# then checks that neither archive contains anything from the game
# (scripts/check-no-game-data.sh) and that the names match the Warpzone
# descriptor's asset globs and exec paths (packaging/warpzone/bopl-port.toml).
#
# Usage: scripts/package.sh [--verify] [--bundle-sdl] [--no-build] [--linux-only | --windows-only]
#   --verify      smoke-test both unpacked archives (scratch/pkgtest/): import
#                 into a data dir in scratch/ and a headless screenshot; the
#                 Windows one under Wine (see the two scripts)
# CPU-heavy (two release builds): run it under perf-mode.
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$BOPL_ROOT"
source scripts/package-common.sh
LIN=1 WIN=1 LARGS=() WARGS=()
for a in "$@"; do
    case $a in
        --verify) LARGS+=(--verify); WARGS+=(--verify) ;;
        --bundle-sdl) LARGS+=(--bundle-sdl) ;;
        --no-build) LARGS+=(--no-build); WARGS+=(--no-build) ;;
        --linux-only) WIN=0 ;;
        --windows-only) LIN=0 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done
VERSION=$(pkg_version)
# Leftovers of other versions would end up in SHA256SUMS.
rm -f "$DIST"/bopl-port-*-linux-x86_64.tar.gz "$DIST"/bopl-port-*-windows-x86_64.zip "$DIST/SHA256SUMS"
[ $LIN = 1 ] && scripts/package-linux.sh "${LARGS[@]}"
[ $WIN = 1 ] && scripts/package-windows.sh "${WARGS[@]}"

# Names and layout against the descriptor: [install] assets and [launch] exec.
python3 -I - "$DIST" "$VERSION" packaging/warpzone/bopl-port.toml <<'PY'
import fnmatch, os, re, sys, tarfile, zipfile
dist, ver, desc = sys.argv[1:]
# The three keys are one-line entries (no tomllib on Python 3.10).
toml = open(desc).read()
def table(key):
    m = re.search(rf"^{key}\s*=\s*\{{(.*)\}}\s*$", toml, re.M)
    return dict(re.findall(r'(\w+)\s*=\s*"([^"]+)"', m.group(1)))
assets, execs = table("assets"), table("exec")
checksums = re.search(r'^checksums\s*=\s*"([^"]+)"', toml, re.M).group(1)
files = sorted(f for f in os.listdir(dist) if f.startswith(f"bopl-port-{ver}-"))
bad = 0
for plat in ("linux", "windows"):
    hits = [f for f in files if fnmatch.fnmatch(f, assets[plat])]
    if not hits:
        continue
    f = hits[0]
    p = os.path.join(dist, f)
    if f.endswith(".zip"):
        names = zipfile.ZipFile(p).namelist()
    else:
        names = [m.name for m in tarfile.open(p).getmembers()]
    ok = [n for n in names if fnmatch.fnmatch(n, execs[plat])]
    print(f"{f}: asset glob {assets[plat]!r} ok, exec {execs[plat]!r} -> {ok[0] if ok else 'MISSING'}")
    bad += not ok
sums = open(os.path.join(dist, checksums)).read()
for f in files:
    if f not in sums:
        print(f"{f} missing from SHA256SUMS"); bad += 1
sys.exit(1 if bad else 0)
PY
scripts/check-no-game-data.sh --archives "$DIST"/bopl-port-"$VERSION"-*
echo
cat "$DIST/SHA256SUMS"
ls -l "$DIST"
