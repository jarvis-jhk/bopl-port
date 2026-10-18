"""Dev-only reference dump: every object of every Unity file of the game as JSON.

Writes ref/dump/<file>.json = {path_id: {"type", "name", "script", "data"}}.
Large binary payloads (image data, audio, meshes) are replaced by their size.
Output stays in ref/ (git-ignored): it is the original game's data.
Usage: python -I scripts/dev/dump_assets.py [file ...]
"""
import json, os, sys
import UnityPy
from UnityPy.helpers.TypeTreeGenerator import TypeTreeGenerator

G = os.environ["BOPL_GAME_DIR"]
D = G + "/BoplBattle_Data/"
OUT = os.path.join(os.environ["BOPL_ROOT"], "ref", "dump")
os.makedirs(OUT, exist_ok=True)

gen = TypeTreeGenerator("2022.3.60f1")
gen.load_local_game(G)

def slim(v):
    if isinstance(v, (bytes, bytearray, memoryview)):
        return {"$bytes": len(v)}
    if isinstance(v, dict):
        return {k: slim(x) for k, x in v.items()}
    if isinstance(v, list):
        if len(v) > 4096 and all(isinstance(x, (int, float)) for x in v[:64]):
            return {"$array": len(v)}
        return [slim(x) for x in v]
    return v

def dump(fname):
    env = UnityPy.load(D + fname)
    env.typetree_generator = gen
    res = {}
    scripts = {}
    for o in env.objects:
        e = {"type": o.type.name}
        try:
            t = o.read_typetree()
            if o.type.name == "MonoBehaviour":
                s = t.get("m_Script", {})
                key = (s.get("m_FileID"), s.get("m_PathID"))
                if key not in scripts:
                    try:
                        sc = o.read(check_read=False).m_Script.read()
                        scripts[key] = sc.m_ClassName
                    except Exception:
                        scripts[key] = None
                e["script"] = scripts[key]
            e["name"] = t.get("m_Name")
            e["data"] = slim(t)
        except Exception as ex:
            e["error"] = str(ex)[:200]
        res[str(o.path_id)] = e
    sf = next((f for f in env.files.values() if hasattr(f, "externals")), None)
    ext = sf.externals if sf else []
    out = {"externals": [x.path for x in ext], "objects": res}
    with open(os.path.join(OUT, fname + ".json"), "w") as f:
        json.dump(out, f, indent=0, default=str)
    return len(res)

names = sys.argv[1:] or sorted(
    n for n in os.listdir(D)
    if n.startswith(("level", "sharedassets", "globalgamemanagers", "resources"))
    and not n.endswith((".resS", ".resource")))
for n in names:
    try:
        print(n, dump(n), flush=True)
    except Exception as ex:
        print(n, "FAILED", ex, flush=True)

# Second pass: resolve MonoBehaviour script class names through the externals
# (UnityPy cannot follow m_Script once a generated type tree is in use).
def resolve_scripts():
    files = {n[:-5]: os.path.join(OUT, n) for n in os.listdir(OUT) if n.endswith(".json")}
    loaded = {b: json.load(open(p)) for b, p in files.items()}
    ms = {}
    for b, d in loaded.items():
        for k, o in d["objects"].items():
            if o["type"] == "MonoScript" and "data" in o:
                ms[(b, k)] = o["data"]["m_ClassName"]
    for b, d in loaded.items():
        changed = False
        for o in d["objects"].values():
            s = o.get("data", {}).get("m_Script") if o["type"] == "MonoBehaviour" else None
            if not s:
                continue
            fid, pid = s["m_FileID"], s["m_PathID"]
            # The generated MonoBehaviour header lacks the align after
            # m_Enabled, so the PPtr is read 3 bytes early: undo that shift.
            if fid & 0xFFFFFF == 0 and fid:
                fid, pid = fid >> 24, pid >> 24
            pid = str(pid)
            if fid > len(d["externals"]):
                continue
            tgt = b if fid == 0 else os.path.basename(d["externals"][fid - 1])
            name = ms.get((tgt, pid))
            if name and o.get("script") != name:
                o["script"] = name
                changed = True
        if changed:
            with open(files[b], "w") as f:
                json.dump(d, f, indent=0, default=str)

resolve_scripts()
