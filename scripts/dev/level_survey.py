"""Dev-only survey of every level scene: prints our own Markdown summary tables.

Reads the reference dump written by scripts/dev/dump_assets.py
(ref/dump/globalgamemanagers.json and ref/dump/level<N>.json, git-ignored) and
prints, for every level scene (build index 6..129):

  * table 1: index, bag index, scene path (shortened), folder group, world
    (from the scene's game-session controller), water box present, platform
    count, platform material types, round platforms (rounded rects with ~zero
    box extents), "free" platforms (home spring speed 0), ability spawners,
    special components;
  * table 2: per-level values that differ from the most common value across
    all levels (game-session timings, camera zoom, physics constants, ...).

Only counts, flags and tuning constants are printed (no geometry, no
positions, no asset content), so the output may be pasted into
docs/spec/levels.md.

Usage:  source env.sh && python -I scripts/dev/level_survey.py [--dump DIR]
"""
import argparse
import collections
import json
import os
import sys

FIRST_LEVEL = 6
ONE = float(1 << 32)
WORLD = {0: "grass", 1: "snow", 2: "space"}
PLATFORM_TYPE = ["grass", "snow", "ice", "space", "robot", "slime"]

# Scripts that make a level "special" (counted per scene).
SPECIAL = [
    "VectorFieldPlatform",
    "AntiLockPlatform",
    "GrowOnStart",
    "AbilitySpawner",
    "KeepInCameraCorner",
    "ParallaxLayer",
    "ScaleBasedOnCameraWidth",
]

# Tuning fields compared across levels: (script, field).
TUNING = [
    ("GameSessionHandler", "teammateSpawnSpacing"),
    ("GameSessionHandler", "levelAnimationRadius"),
    ("GameSessionHandler", "MaxRandomDelay"),
    ("GameSessionHandler", "timeBeforeGO"),
    ("GameSessionHandler", "timeBeforeTwo"),
    ("GameSessionHandler", "DelayBeforeNextLevelInput"),
    ("GameSessionHandler", "TimeBeforeSuddenDeath"),
    ("GameSessionHandler", "LevelAnimationTime"),
    ("GameSessionHandler", "blockAnimateInDelay_constantDelayFactor"),
    ("GameSessionHandler", "blockAnimateOutDelay_constantDelayFactor"),
    ("GameSessionHandler", "startProgressOfAnimateInGrounds"),
    ("GameSessionHandler", "musicTrack"),
    ("PlayerAverageCamera", "MIN_ZOOM"),
    ("PlayerAverageCamera", "MAX_ZOOM"),
    ("PlayerAverageCamera", "weight"),
    ("PlayerAverageCamera", "zoomWeight"),
    ("PlayerAverageCamera", "MinHeightAboveFloor"),
    ("PlayerAverageCamera", "UpdateX"),
    ("PlayerAverageCamera", "UpdateY"),
    ("PlayerAverageCamera", "useMinMaxForCameraX"),
    ("PlayerAverageCamera", "outsideLevelX"),
    ("PlayerAverageCamera", "extraZoomRoom"),
    ("ShakableCamera", "maxOffset"),
    ("ShakableCamera", "shakeX"),
    ("ShakableCamera", "shakeY"),
    ("DetPhysics", "gravityAccel"),
    ("DetPhysics", "playerGravity"),
    ("DetPhysics", "ropeGravity"),
    ("DetPhysics", "platformStretchiness"),
    ("DetPhysics", "maxBeamDistance"),
    ("SceneBoundArrowController", "ArrowOffset"),
    ("Updater", "autoUpdate"),
]


def fix(v):
    """Q32.32 raw value (as stored in the dump) -> float, rounded for display."""
    if isinstance(v, dict) and "m_rawValue" in v:
        return round(v["m_rawValue"] / ONE, 4)
    if isinstance(v, float):
        return round(v, 4)
    return v


def load(path):
    with open(path) as f:
        return json.load(f)


def scene_list(dump):
    gg = load(os.path.join(dump, "globalgamemanagers.json"))["objects"]
    for v in gg.values():
        if v.get("type") == "BuildSettings":
            return v["data"]["scenes"]
    sys.exit("BuildSettings not found in globalgamemanagers.json")


def short(path):
    p = path.replace("Assets/Scenes/", "").replace(".unity", "")
    return p


def group(path):
    p = short(path)
    if p.startswith("extraLevels/"):
        return "extra"
    if p.startswith("2v2/"):
        return "2v2"
    return "FFA"


def by_script(objs):
    out = collections.defaultdict(list)
    for v in objs.values():
        if v.get("type") == "MonoBehaviour":
            out[str(v.get("script"))].append(v.get("data", {}))
    return out


def game_object(objs, comp_data):
    ref = comp_data.get("m_GameObject", {})
    return objs.get(str(ref.get("m_PathID")), {}).get("data", {})


def survey_level(objs):
    s = by_script(objs)
    row = {}
    gsh = s.get("GameSessionHandler", [{}])[0]
    row["world"] = WORLD.get(gsh.get("levelType"), "?")
    plats = s.get("StickyRoundedRectangle", [])
    row["platforms"] = len(plats)
    types = collections.Counter(PLATFORM_TYPE[p.get("platformType", 0)] for p in plats)
    row["types"] = " ".join(f"{k}{'×%d' % n if n > 1 else ''}" for k, n in sorted(types.items()))
    # Round platforms: rounded rects whose half extents are (almost) zero.
    rounds = 0
    falling = 0
    for rr in s.get("DPhysicsRoundedRect", []):
        ex = rr.get("startExtents", {})
        if abs(fix(ex.get("x", {"m_rawValue": 0}))) < 0.05 and abs(fix(ex.get("y", {"m_rawValue": 0}))) < 0.05:
            rounds += 1
    for b in s.get("BoplBody", []):
        if fix(b.get("gravityScale", {"m_rawValue": 0})) != 0:
            falling += 1
    row["round"] = rounds
    row["gravity_bodies"] = falling
    still = sum(1 for a in s.get("AnimateVelocity", []) if fix(a.get("speed")) == 0)
    row["no_spring"] = still
    fixed_home = sum(1 for a in s.get("AnimateVelocity", []) if not a.get("HomeIsMovable", 1))
    row["fixed_home"] = fixed_home
    water = False
    for b in s.get("DPhysicsBox", []):
        g = game_object(objs, b)
        if g.get("m_Layer") == 4:
            water = True
    row["water"] = water
    sp = []
    for name in SPECIAL:
        n = len(s.get(name, []))
        if name == "AbilitySpawner":
            row["spawners"] = n
            continue
        if name in ("ParallaxLayer", "ScaleBasedOnCameraWidth"):
            continue
        if n:
            sp.append(f"{name}×{n}" if n > 1 else name)
    row["special"] = ", ".join(sp)
    row["parallax"] = len(s.get("ParallaxLayer", []))
    tun = {}
    for script, field in TUNING:
        lst = s.get(script)
        if lst and field in lst[0]:
            tun[(script, field)] = fix(lst[0][field])
    row["tuning"] = tun
    return row


def main():
    ap = argparse.ArgumentParser()
    root = os.environ.get("BOPL_ROOT", os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    ap.add_argument("--dump", default=os.path.join(root, "ref", "dump"))
    a = ap.parse_args()
    scenes = scene_list(a.dump)
    rows = []
    for idx in range(FIRST_LEVEL, len(scenes)):
        objs = load(os.path.join(a.dump, f"level{idx}.json"))["objects"]
        r = survey_level(objs)
        r["idx"] = idx
        r["bag"] = idx - FIRST_LEVEL
        r["path"] = short(scenes[idx])
        r["group"] = group(scenes[idx])
        rows.append(r)

    print("| scene | bag | path | group | world | water | platforms | types | round | free | spawners | special |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for r in rows:
        print(
            f"| {r['idx']} | {r['bag']} | `{r['path']}` | {r['group']} | {r['world']} | "
            f"{'yes' if r['water'] else 'no'} | {r['platforms']} | {r['types']} | {r['round'] or ''} | "
            f"{r['no_spring'] or ''} | {r['spawners']} | {r['special']} |"
        )

    # Summary counts.
    print()
    tot = collections.Counter()
    for r in rows:
        tot[(r["group"], r["world"])] += 1
    print("| group | world | levels |")
    print("|---|---|---|")
    for (g, w), n in sorted(tot.items()):
        print(f"| {g} | {w} | {n} |")
    print(f"| all | all | {len(rows)} |")
    print()
    print(f"platforms total: {sum(r['platforms'] for r in rows)}, "
          f"min {min(r['platforms'] for r in rows)}, max {max(r['platforms'] for r in rows)}; "
          f"round {sum(r['round'] for r in rows)}, free {sum(r['no_spring'] for r in rows)}, "
          f"with gravity at start {sum(r['gravity_bodies'] for r in rows)}, "
          f"fixed home {sum(r['fixed_home'] for r in rows)}")

    # Tuning: modal value and the levels deviating from it.
    print()
    print("| component.field | value in most levels | levels with another value |")
    print("|---|---|---|")
    for key in TUNING:
        vals = [(r["idx"], r["tuning"].get(key)) for r in rows]
        present = [(i, v) for i, v in vals if v is not None]
        if not present:
            continue
        mode, _ = collections.Counter(json.dumps(v) for _, v in present).most_common(1)[0]
        mode_v = json.loads(mode)
        dev = collections.defaultdict(list)
        for i, v in present:
            if json.dumps(v) != mode:
                dev[json.dumps(v)].append(i)
        dev_s = "; ".join(f"{json.loads(k)!r}: {compress(v)}" for k, v in sorted(dev.items())) or "-"
        missing = [i for i, v in vals if v is None]
        if missing:
            dev_s += f" (absent in {compress(missing)})"
        print(f"| {key[0]}.{key[1]} | {mode_v!r} | {dev_s} |")


def compress(idxs):
    """[6,7,8,10] -> '6-8, 10'"""
    idxs = sorted(idxs)
    out = []
    start = prev = None
    for i in idxs:
        if start is None:
            start = prev = i
        elif i == prev + 1:
            prev = i
        else:
            out.append(f"{start}-{prev}" if prev != start else f"{start}")
            start = prev = i
    if start is not None:
        out.append(f"{start}-{prev}" if prev != start else f"{start}")
    return ", ".join(out)


if __name__ == "__main__":
    main()
