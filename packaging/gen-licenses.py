#!/usr/bin/env python3
"""Writes the licenses of every Rust crate linked into bopl-port.

Usage: python -I packaging/gen-licenses.py OUT.txt [TARGET_TRIPLE]
(run from the workspace root, after `source env.sh`).

Follows normal (non-dev, non-build) dependencies of the `bopl-port` package
for TARGET_TRIPLE through `cargo metadata`, then writes, for each distinct
license text found in the crates' source directories (LICENSE*, COPYING*,
NOTICE*, ...), the crates it belongs to followed by the text. Crates of this
workspace are skipped (their license is the project's own). Prints a summary
of license expressions and exits 1 if one looks like copyleft that needs
attention (GPL/AGPL/LGPL without an alternative).
"""
import hashlib
import json
import os
import re
import subprocess
import sys

out_path = sys.argv[1]
triple = sys.argv[2] if len(sys.argv) > 2 else "x86_64-unknown-linux-gnu"
meta = json.loads(
    subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", triple],
        check=True, capture_output=True, text=True,
    ).stdout
)
pkgs = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
root = next(p["id"] for p in meta["packages"] if p["name"] == "bopl-port")
workspace = set(meta["workspace_members"])

seen, stack = set(), [root]
while stack:
    i = stack.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]["deps"]:
        if any(k.get("kind") is None for k in d["dep_kinds"]):
            stack.append(d["pkg"])

LICENSE_RE = re.compile(r"^(licen[sc]e|copying|notice|unlicense|copyright)", re.I)
texts = {}  # sha -> (text, [crates])
exprs = {}  # expression -> [crates]
missing = []
for i in sorted(seen, key=lambda i: (pkgs[i]["name"], pkgs[i]["version"])):
    if i in workspace:
        continue
    p = pkgs[i]
    label = f'{p["name"]} {p["version"]}'
    exprs.setdefault(p.get("license") or "(none)", []).append(label)
    d = os.path.dirname(p["manifest_path"])
    found = False
    for f in sorted(os.listdir(d)):
        fp = os.path.join(d, f)
        if os.path.isfile(fp) and LICENSE_RE.match(f) and not f.endswith((".rs", ".toml")):
            raw = open(fp, "rb").read().decode("utf-8", "replace").strip()
            if not raw:
                continue
            h = hashlib.sha256(re.sub(r"\s+", " ", raw).encode()).hexdigest()
            texts.setdefault(h, (raw, []))[1].append(label)
            found = True
    if not found:
        missing.append(label)

with open(out_path, "w") as o:
    o.write("Licenses of the Rust crates linked into bopl-port\n")
    o.write(f"({len(seen) - len(workspace & seen)} crates, target {triple})\n\n")
    o.write("License expressions as declared by the crates:\n\n")
    for e, cs in sorted(exprs.items()):
        o.write(f"  {e}  ({len(cs)} crates)\n")
    o.write("\nCrates without a license file in their source (license per the expression above):\n")
    o.write("  " + (", ".join(missing) if missing else "none") + "\n")
    for text, crates in sorted(texts.values(), key=lambda t: t[1][0]):
        o.write("\n" + "=" * 78 + "\n")
        o.write("Used by: " + ", ".join(crates) + "\n")
        o.write("=" * 78 + "\n\n" + text + "\n")

print(f"{out_path}: {len(seen) - len(workspace & seen)} crates, {len(texts)} distinct license texts")
bad = [e for e in exprs if re.search(r"GPL", e) and "OR" not in e]
for e in sorted(exprs):
    print(f"  {len(exprs[e]):4}  {e}")
if bad:
    print("copyleft needs attention:", bad, file=sys.stderr)
    sys.exit(1)
