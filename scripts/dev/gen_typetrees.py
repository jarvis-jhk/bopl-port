"""Dev-only: generate the type trees that `unity-assets` embeds.

The build ships without type trees, so the reader needs the node layouts of
the engine classes (from UnityPy's TPK database for 2022.3.60f1) and of the
game's MonoBehaviour classes (from TypeTreeGenerator run over the game's
managed DLLs). Only layouts (field names, types, alignment flags) are written,
no data from the game's assets:

  crates/unity/typetrees/engine.json   every engine class used by the game files
  crates/unity/typetrees/scripts.json  every MonoBehaviour class used by them

Script trees omit the MonoBehaviour header (m_GameObject .. m_Name): the reader
prepends the engine header, which (unlike the generator's) aligns after
m_Enabled.

Usage: python -I scripts/dev/gen_typetrees.py
"""
import json
import os
import sys

import UnityPy
from UnityPy.helpers.Tpk import get_typetree_node
from UnityPy.helpers.TypeTreeGenerator import TypeTreeGenerator
from UnityPy.helpers.UnityVersion import UnityVersion

VERSION = "2022.3.60f1"
G = os.environ["BOPL_GAME_DIR"]
D = os.path.join(G, "BoplBattle_Data")
OUT = os.path.join(os.environ["BOPL_ROOT"], "crates", "unity", "typetrees")

# Engine classes the importer needs even if a given install lacks them.
REQUIRED = [1, 4, 5, 19, 20, 21, 28, 48, 49, 50, 58, 60, 61, 62, 66, 68, 70, 74, 78, 82,
            83, 91, 95, 96, 114, 115, 120, 128, 141, 198, 199, 210, 212, 213, 222,
            223, 224, 225, 231, 232, 233, 234, 235, 249, 250, 251, 252, 253, 254,
            255, 256, 257, 331]

# Plain [Serializable] classes stored through [SerializeReference] (payloads of
# ManagedReferencesRegistry). Without embedded type trees their names are only
# visible in the data; this list is maintained by running
# `cargo run -p unity-assets --example validate`, which reports unknown ones.
REF_TYPES = []

HEADER = ["m_GameObject", "m_FileID", "m_PathID", "m_Enabled", "m_Script",
          "m_FileID", "m_PathID", "m_Name"]


def game_files():
    names = sorted(
        n for n in os.listdir(D)
        if n.startswith(("level", "sharedassets", "globalgamemanagers", "resources"))
        and not n.endswith((".resS", ".resource")))
    res = os.path.join(D, "Resources")
    return [os.path.join(D, n) for n in names] + [
        os.path.join(res, n) for n in sorted(os.listdir(res))]


def tpk_nodes(cid, version):
    root = get_typetree_node(cid, version)
    return [[n.m_Level, n.m_Type, n.m_Name, n.m_ByteSize, n.m_MetaFlag]
            for n in root.traverse()]


def main():
    version = UnityVersion.from_str(VERSION)
    env = UnityPy.load(*game_files())
    class_ids = set(REQUIRED)
    scripts = set()
    for path, f in env.files.items():
        if not hasattr(f, "objects"):
            continue
        for o in f.objects.values():
            class_ids.add(o.class_id)
            if o.class_id != 114:
                continue
            try:
                head = o.parse_monobehaviour_head()
                sc = head.m_Script.deref_parse_as_dict()
            except Exception as ex:  # unresolved script reference
                print("skip", path, o.path_id, ex, file=sys.stderr)
                continue
            scripts.add((sc["m_AssemblyName"], sc["m_Namespace"], sc["m_ClassName"]))

    engine = []
    for cid in sorted(class_ids):
        try:
            nodes = tpk_nodes(cid, version)
        except Exception as ex:
            print("no engine tree for class", cid, ex, file=sys.stderr)
            continue
        engine.append({"id": cid, "name": nodes[0][1], "nodes": nodes})

    gen = TypeTreeGenerator(VERSION)
    gen.load_local_game(G)
    out_scripts = []
    for asm, ns, cls in sorted(scripts):
        fields = script_fields(gen, asm, ns, cls)
        if fields is not None:
            out_scripts.append({"assembly": asm.removesuffix(".dll"), "namespace": ns,
                                "class": cls, "nodes": fields})

    out_refs = []
    for asm, ns, cls in REF_TYPES:
        fields = script_fields(gen, asm, ns, cls)
        if fields is not None:
            out_refs.append({"assembly": asm, "namespace": ns, "class": cls, "nodes": fields})

    os.makedirs(OUT, exist_ok=True)
    write(os.path.join(OUT, "engine.json"),
          {"unity_version": VERSION,
           "source": "UnityPy TPK (engine class database)",
           "node": ["level", "type", "name", "byte_size", "meta_flags"]},
          {"classes": engine})
    write(os.path.join(OUT, "scripts.json"),
          {"unity_version": VERSION,
           "source": "TypeTreeGenerator over the game's managed assemblies; "
                     "fields after the MonoBehaviour header",
           "node": ["level", "type", "name", "meta_flags"]},
          {"scripts": out_scripts, "ref_types": out_refs})
    print(f"{len(engine)} engine classes, {len(out_scripts)} script classes, "
          f"{len(out_refs)} reference types")


def script_fields(gen, asm, ns, cls):
    """Generated fields of a script class after the MonoBehaviour header."""
    full = f"{ns}.{cls}" if ns else cls
    dll = asm if asm.endswith(".dll") else asm + ".dll"
    try:
        nodes = gen.get_nodes(dll, full)
    except Exception as ex:
        print("no script tree for", asm, full, ex, file=sys.stderr)
        return None
    if not nodes:
        print("empty script tree for", asm, full, file=sys.stderr)
        return None
    names = [n.m_Name for n in nodes[1:1 + len(HEADER)]]
    if names != HEADER:
        print("unexpected header for", asm, full, names, file=sys.stderr)
        return None
    return [[n.m_Level, n.m_Type, n.m_Name, n.m_MetaFlag] for n in nodes[1 + len(HEADER):]]


def write(path, head, lists):
    """One item per line: compact but still diffable."""
    with open(path, "w") as f:
        f.write("{\n")
        for k, v in head.items():
            f.write(f"{json.dumps(k)}: {json.dumps(v)},\n")
        for j, (key, items) in enumerate(lists.items()):
            f.write(f"{json.dumps(key)}: [\n")
            for i, it in enumerate(items):
                sep = "," if i + 1 < len(items) else ""
                f.write(json.dumps(it, separators=(",", ":")) + sep + "\n")
            f.write("]" + ("," if j + 1 < len(lists) else "") + "\n")
        f.write("}\n")


if __name__ == "__main__":
    main()
