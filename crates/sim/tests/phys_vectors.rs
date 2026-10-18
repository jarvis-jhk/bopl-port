//! Narrow phase, ray casts and shape helpers against the original's own
//! code (`scratch/fixoracle/vectors.json`, `phys.*`). Skipped when absent.

use bopl_sim::fix::{Fix, take_fault};
use bopl_sim::physics::shapes::*;
use bopl_sim::vec2::Vec2;
use serde_json::Value as J;

/// `scratch/fixoracle/vectors.json` of this checkout, else of the main
/// checkout named by `BOPL_ROOT` (git worktrees have no `scratch/`).
fn vectors() -> Option<J> {
    let rel = "scratch/fixoracle/vectors.json";
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let root = std::env::var_os("BOPL_ROOT").map(|r| std::path::PathBuf::from(r).join(rel));
    let p = std::iter::once(here).chain(root).find(|p| p.exists())?;
    let s = std::fs::read(&p).ok()?;
    Some(serde_json::from_slice(&s).unwrap())
}

fn i(v: &J) -> i64 {
    v.as_i64()
        .or_else(|| v.as_u64().map(|u| u as i64))
        .unwrap_or_else(|| panic!("not an int: {v}"))
}
fn f(v: &J) -> Fix {
    Fix(i(v))
}
fn v2(v: &J) -> Vec2 {
    Vec2::raw(i(&v["x"]), i(&v["y"]))
}
fn circle(v: &J) -> Circle {
    Circle {
        center: v2(&v["center"]),
        radius: f(&v["radius"]),
        layer: i(&v["layer"]) as i32,
    }
}
fn bx(v: &J) -> BoxShape {
    BoxShape {
        center: v2(&v["center"]),
        right: v2(&v["right"]),
        up: v2(&v["up"]),
        ie: v2(&v["inverseExtents"]),
        layer: i(&v["layer"]) as i32,
    }
}
fn rr(v: &J) -> RoundedRect {
    RoundedRect {
        b: bx(&v["box"]),
        radius: f(&v["radius"]),
        layer: i(&v["layer"]) as i32,
    }
}
fn aabb(v: &J) -> Aabb {
    Aabb {
        min: v2(&v["min"]),
        max: v2(&v["max"]),
        id: i(&v["id"]) as i32,
        layer: i(&v["layer"]) as i32,
        shape: i(&v["shape"]) as u8,
    }
}

fn check_manifold(name: &str, k: usize, row: &J, got: Option<Manifold>, bad: &mut usize) {
    let fault = take_fault();
    if let Some(t) = row["hit"].get("throws") {
        if fault.is_none() {
            *bad += 1;
            eprintln!("{name}[{k}]: expected {t}");
        }
        return;
    }
    let hit = row["hit"].as_bool().unwrap();
    let ok = match (&got, hit) {
        (None, false) => true,
        (Some(m), true) => {
            let e = &row["manifold"];
            m.penetration.0 == i(&e["penetration"])
                && m.normal == v2(&e["normal"])
                && m.contact == v2(&e["contactPoint"])
        }
        _ => false,
    };
    if !ok || fault.is_some() {
        *bad += 1;
        if *bad < 10 {
            eprintln!(
                "{name}[{k}]: expected hit={hit} {}, got {got:?} fault {fault:?}",
                row["manifold"]
            );
        }
    }
}

#[test]
fn narrow_phase() {
    let Some(d) = vectors() else {
        eprintln!("vectors.json not present; skipping");
        return;
    };
    let p = &d["phys"];
    let mut bad = 0;
    let mut bool_bad = 0;
    let mut total = 0;
    for (name, test) in [
        "collide_circle_circle",
        "collide_circle_box",
        "collide_circle_rr",
        "collide_box_box",
        "collide_box_rr",
        "collide_rr_rr",
    ]
    .iter()
    .map(|n| (*n, &p[*n]))
    {
        let b0 = bad;
        for (k, row) in test.as_array().unwrap().iter().enumerate() {
            let (a, b) = (&row["a"], &row["b"]);
            let (got, gb) = match name {
                "collide_circle_circle" => (
                    circle_circle(&circle(a), &circle(b)),
                    circle_circle_bool(&circle(a), &circle(b)),
                ),
                "collide_circle_box" => (
                    circle_box(&circle(a), &bx(b)),
                    circle_box_bool(&circle(a), &bx(b)),
                ),
                "collide_circle_rr" => (
                    circle_rr(&circle(a), &rr(b)),
                    circle_rr_bool(&circle(a), &rr(b)),
                ),
                "collide_box_box" => (box_box(&bx(a), &bx(b)), box_box_bool(&bx(a), &bx(b))),
                "collide_box_rr" => (box_rr(&bx(a), &rr(b)), box_rr_bool(&bx(a), &rr(b))),
                _ => (rr_rr(&rr(a), &rr(b)), rr_rr_bool(&rr(a), &rr(b))),
            };
            check_manifold(name, k, row, got, &mut bad);
            if let Some(bv) = row.get("bool_version").and_then(|v| v.as_bool()) {
                let _ = take_fault();
                if bv != gb {
                    bool_bad += 1;
                    if bool_bad < 10 {
                        eprintln!("{name}[{k}] bool: expected {bv}, got {gb}");
                    }
                }
            }
            total += 1;
        }
        eprintln!("{name}: {} mismatches", bad - b0);
    }
    eprintln!("{total} narrow-phase vectors, {bad} mismatches, {bool_bad} bool mismatches");
    assert_eq!(bad + bool_bad, 0);
}

#[test]
fn raycasts() {
    let Some(d) = vectors() else {
        return;
    };
    let p = &d["phys"];
    let mut bad = 0;
    let mut n = 0;
    for name in ["raycast_rr", "raycast_circle", "raycast_box"] {
        for (k, row) in p[name].as_array().unwrap().iter().enumerate() {
            let o = v2(&row["origin"]);
            let dir = v2(&row["dir"]);
            let got = match name {
                "raycast_rr" => ray_rr(o, dir, &rr(&row["rr"])),
                "raycast_circle" => ray_circle(o, dir, &circle(&row["circle"])),
                _ => ray_box(o, dir, &bx(&row["box"])),
            };
            let _ = take_fault();
            if row["hit"].get("throws").is_some() {
                continue; // the oracle cannot run this one (Unity debug drawing)
            }
            n += 1;
            let hit = row["hit"].as_bool().unwrap();
            let ok = match got {
                None => !hit,
                Some((near, far)) => hit && near.0 == i(&row["near"]) && far.0 == i(&row["far"]),
            };
            if !ok {
                bad += 1;
                if bad < 10 {
                    eprintln!(
                        "{name}[{k}]: expected hit={hit} near {} far {}, got {got:?}",
                        row["near"], row["far"]
                    );
                }
            }
        }
    }
    eprintln!("{n} ray vectors, {bad} mismatches");
    assert_eq!(bad, 0);
}

#[test]
fn shape_queries() {
    let Some(d) = vectors() else {
        return;
    };
    let mut bad = 0;
    for (k, row) in d["phys"]["shape_queries"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let r = rr(&row["rr"]);
        let b = bx(&row["box"]);
        let c = circle(&row["circle"]);
        let pt = v2(&row["p"]);
        let mut chk = |what: &str, ok: bool| {
            let _ = take_fault();
            if !ok {
                bad += 1;
                if bad < 10 {
                    eprintln!("shape_queries[{k}] {what} mismatch");
                }
            }
        };
        chk("rr_closest", r.closest_point(pt) == v2(&row["rr_closest"]));
        chk(
            "rr_overlap",
            r.contains(pt) == row["rr_overlap"].as_bool().unwrap(),
        );
        chk(
            "box_overlap",
            b.contains(pt) == row["box_overlap"].as_bool().unwrap(),
        );
        chk(
            "circle_overlap",
            c.contains(pt) == row["circle_overlap"].as_bool().unwrap(),
        );
        chk("rr_normal", r.normal_at(pt) == v2(&row["rr_normal"]));
        chk("box_normal", b.normal_at(pt) == v2(&row["box_normal"]));
        let id = i(&row["rr_aabb"]["id"]) as i32;
        chk("rr_aabb", r.aabb(id) == aabb(&row["rr_aabb"]));
        chk("box_aabb", b.aabb(id) == aabb(&row["box_aabb"]));
        chk("circle_aabb", c.aabb(id) == aabb(&row["circle_aabb"]));
    }
    assert_eq!(bad, 0);
}

#[test]
fn positional_correction_vectors() {
    let Some(d) = vectors() else {
        return;
    };
    let mut bad = 0;
    for (k, row) in d["phys"]["resolve_collision"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let Some(pc) = row.get("positional_correction") else {
            continue;
        };
        let m = &row["manifold"];
        let man = Manifold {
            penetration: f(&m["penetration"]),
            normal: v2(&m["normal"]),
            contact: v2(&m["contactPoint"]),
        };
        let ima = f(&row["bodyA"]["inverseMass"]);
        let imb = f(&row["bodyB"]["inverseMass"]);
        let fault_expected = pc.get("throws").is_some();
        let (a, b) = positional_correction_std(&man, ima, imb);
        let fault = take_fault();
        let ok = if fault_expected {
            fault.is_some()
        } else {
            fault.is_none() && a == v2(&pc["a"]) && b == v2(&pc["b"])
        };
        if !ok {
            bad += 1;
            if bad < 10 {
                eprintln!("pc[{k}]: expected {pc}, got {a:?} {b:?} {fault:?}");
            }
        }
    }
    assert_eq!(bad, 0);
}
