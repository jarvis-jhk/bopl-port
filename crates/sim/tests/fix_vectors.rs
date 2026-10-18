//! Golden vectors produced by running the original's own math code
//! (`scratch/fixoracle/vectors.json`, git-ignored). Skipped when absent.

use bopl_sim::fix::{Fault, Fix, take_fault};
use bopl_sim::rng::Rng;
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

macro_rules! need {
    () => {
        match vectors() {
            Some(v) => v,
            None => {
                eprintln!("vectors.json not present; skipping");
                return;
            }
        }
    };
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
    if let Some(a) = v.as_array() {
        Vec2::raw(i(&a[0]), i(&a[1]))
    } else {
        Vec2::raw(i(&v["x"]), i(&v["y"]))
    }
}

/// Expected outcome: a value or the name of the exception.
enum Exp<'a> {
    Val(&'a J),
    Throws(&'a str),
}

fn exp(v: &J) -> Exp<'_> {
    match v.get("throws") {
        Some(t) => Exp::Throws(t.as_str().unwrap()),
        None => Exp::Val(v),
    }
}

fn fault_name(f: Fault) -> &'static str {
    match f {
        Fault::DivideByZero => "DivideByZeroException",
        Fault::Overflow => "OverflowException",
        Fault::Argument => "ArgumentOutOfRangeException",
        Fault::Index => "IndexOutOfRangeException",
    }
}

/// Checks one scalar result against the expectation, counting mismatches.
fn check(op: &str, input: &str, got: Fix, e: &J, bad: &mut usize) {
    let fault = take_fault();
    match exp(e) {
        Exp::Throws(t) => {
            if fault.map(fault_name) != Some(t) {
                *bad += 1;
                if *bad < 20 {
                    eprintln!("{op}({input}): expected {t}, got {got:?} fault {fault:?}");
                }
            }
        }
        Exp::Val(v) => {
            if fault.is_some() || got.0 != i(v) {
                *bad += 1;
                if *bad < 20 {
                    eprintln!(
                        "{op}({input}): expected {}, got {} fault {fault:?}",
                        i(v),
                        got.0
                    );
                }
            }
        }
    }
}

fn check_v(op: &str, input: &str, got: Vec2, e: &J, bad: &mut usize) {
    let fault = take_fault();
    match exp(e) {
        Exp::Throws(t) => {
            if fault.map(fault_name) != Some(t) {
                *bad += 1;
                eprintln!("{op}({input}): expected {t}, got {got:?}");
            }
        }
        Exp::Val(v) => {
            if fault.is_some() || got != v2(v) {
                *bad += 1;
                if *bad < 20 {
                    eprintln!("{op}({input}): expected {:?}, got {got:?}", v2(v));
                }
            }
        }
    }
}

#[test]
fn fix_binary_ops() {
    let d = need!();
    let mut total = 0;
    let mut bad = 0;
    type Op = fn(Fix, Fix) -> Fix;
    let ops: &[(&str, Op)] = &[
        ("add", |a, b| a + b),
        ("sub", |a, b| a - b),
        ("mul", |a, b| a * b),
        ("div", |a, b| a / b),
        ("mod", |a, b| a % b),
        ("slow_add", |a, b| a.slow_add(b)),
        ("slow_sub", |a, b| a.slow_sub(b)),
        ("slow_mul", |a, b| a.slow_mul(b)),
        ("slow_mod", |a, b| a.slow_mod(b)),
        ("min", |a, b| a.min(b)),
        ("max", |a, b| a.max(b)),
        ("atan2", |y, x| Fix::atan2(y, x)),
        ("pow", |b, e| b.pow(e)),
    ];
    for (name, op) in ops {
        for row in d["fix"][*name].as_array().unwrap() {
            let a = f(&row[0]);
            let b = f(&row[1]);
            check(
                name,
                &format!("{}, {}", a.0, b.0),
                op(a, b),
                &row[2],
                &mut bad,
            );
            total += 1;
        }
    }
    for row in d["fix"]["lerp"].as_array().unwrap() {
        let got = f(&row[0]).lerp(f(&row[1]), f(&row[2]));
        check("lerp", &row.to_string(), got, &row[3], &mut bad);
        total += 1;
    }
    eprintln!("{total} binary vectors, {bad} mismatches");
    assert_eq!(bad, 0);
}

#[test]
fn fix_unary_ops() {
    let d = need!();
    let mut total = 0;
    let mut bad = 0;
    type Op = fn(Fix) -> Fix;
    let ops: &[(&str, Op)] = &[
        ("neg", |a| -a),
        ("abs", |a| a.abs()),
        ("slow_abs", |a| a.slow_abs()),
        ("floor", |a| a.floor()),
        ("ceiling", |a| a.ceiling()),
        ("round", |a| a.round()),
        ("sqrt", |a| a.sqrt()),
        ("sin", |a| a.sin()),
        ("fast_sin", |a| a.fast_sin()),
        ("cos", |a| a.cos()),
        ("fast_cos", |a| a.fast_cos()),
        ("tan", |a| a.tan()),
        ("atan", |a| a.atan()),
        ("acos", |a| a.acos()),
        ("log2", |a| a.log2()),
        ("ln", |a| a.ln()),
        ("pow2", |a| a.pow2()),
        ("clamp01", |a| a.clamp01()),
        ("to_long", |a| Fix(a.to_long())),
    ];
    for (name, op) in ops {
        let mut n = 0;
        let b0 = bad;
        for row in d["fix"][*name].as_array().unwrap() {
            let a = f(&row[0]);
            check(name, &a.0.to_string(), op(a), &row[1], &mut bad);
            n += 1;
        }
        eprintln!("{name}: {n} vectors, {} mismatches", bad - b0);
        total += n;
    }
    for row in d["fix"]["sign_sign2"].as_array().unwrap() {
        let a = f(&row[0]);
        assert_eq!(a.sign() as i64, i(&row[1]), "sign {}", a.0);
        assert_eq!(a.sign2() as i64, i(&row[2]), "sign2 {}", a.0);
        total += 1;
    }
    eprintln!("{total} unary vectors, {bad} mismatches");
    assert_eq!(bad, 0);
}

#[test]
fn fix_conversions() {
    let d = need!();
    let fx = &d["fix"];
    for row in fx["from_long"].as_array().unwrap() {
        assert_eq!(Fix::from_long(i(&row[0])).0, i(&row[1]), "from_long {row}");
    }
    for row in fx["ctor_int"].as_array().unwrap() {
        assert_eq!(Fix::from_int(i(&row[0]) as i32).0, i(&row[1]), "ctor {row}");
    }
    for row in fx["from_float"].as_array().unwrap() {
        let bits = i(&row[0]) as u32;
        let got = Fix::from_f32(f32::from_bits(bits));
        assert_eq!(got.0, i(&row[2]), "from_float {row}");
    }
    for row in fx["from_double"].as_array().unwrap() {
        let bits = i(&row[0]) as u64;
        let got = Fix::from_f64(f64::from_bits(bits));
        assert_eq!(got.0, i(&row[2]), "from_double {row}");
    }
    for row in fx["from_decimal"].as_array().unwrap() {
        let got = Fix::from_decimal_str(row[0].as_str().unwrap()).unwrap();
        assert_eq!(got.0, i(&row[1]), "from_decimal {row}");
    }
    for row in fx["to_float_double"].as_array().unwrap() {
        let a = f(&row[0]);
        assert_eq!(
            a.to_f32().to_bits() as i64 as i32,
            i(&row[1]["f32bits"]) as i32,
            "f32 {row}"
        );
        assert_eq!(
            a.to_f64().to_bits() as i64,
            i(&row[2]["f64bits"]),
            "f64 {row}"
        );
    }
}

#[test]
fn constants() {
    let d = need!();
    let c = &d["constants"];
    let k = |n: &str| i(&c[n]);
    assert_eq!(Fix::ONE.0, k("BoplFixedMath.Fix.One"));
    assert_eq!(Fix::PI.0, k("BoplFixedMath.Fix.Pi"));
    assert_eq!(Fix::PI_OVER_2.0, k("BoplFixedMath.Fix.PiOver2"));
    assert_eq!(Fix::PI_TIMES_2.0, k("BoplFixedMath.Fix.PiTimes2"));
    assert_eq!(Fix::PI_INV.0, k("BoplFixedMath.Fix.PiInv"));
    assert_eq!(Fix::PI_OVER_2_INV.0, k("BoplFixedMath.Fix.PiOver2Inv"));
    assert_eq!(Fix::LN2.0, k("BoplFixedMath.Fix.Ln2"));
    assert_eq!(Fix::LOG2_MAX.0, k("BoplFixedMath.Fix.Log2Max"));
    assert_eq!(Fix::LOG2_MIN.0, k("BoplFixedMath.Fix.Log2Min"));
    assert_eq!(Fix::LUT_INTERVAL.0, k("BoplFixedMath.Fix.LutInterval"));
    use bopl_sim::consts as K;
    assert_eq!(K::FIXED_DT.0, k("GameTime.FixedTimeStep"));
    assert_eq!(K::STANDARD_FIXED_DT.0, k("Updater.standardFixedDeltaTime"));
    assert_eq!(K::CAMERA_X_MIN.0, k("SceneBounds.Camera_XMin"));
    assert_eq!(K::CAMERA_X_MAX.0, k("SceneBounds.Camera_XMax"));
    assert_eq!(K::CAMERA_Y_MAX.0, k("SceneBounds.Camera_YMax"));
    assert_eq!(K::CAMERA_Y_MIN.0, k("SceneBounds.Camera_YMin"));
    assert_eq!(K::WATER_HEIGHT.0, k("SceneBounds.waterHeight"));
    assert_eq!(K::SPACE_WATER_HEIGHT.0, k("SceneBounds.spaceWaterHeight"));
    assert_eq!(K::BLAST_X_MIN.0, k("SceneBounds.BlastZone_XMin"));
    assert_eq!(K::BLAST_X_MAX.0, k("SceneBounds.BlastZone_XMax"));
    assert_eq!(K::BLAST_Y_MAX.0, k("SceneBounds.BlastZone_YMax"));
    assert_eq!(
        K::GROUND_ALIGNMENT_DEADZONE.0,
        k("Constants.GROUNDALIGNMENTDEADZONE")
    );
    assert_eq!(K::DEADZONE.0, k("Constants.DEADZONE"));
    assert_eq!(
        K::ENVIRONMENT_ID as i64,
        k("Constants.ENVIRONMENTAL_PLAYER_ID")
    );
    assert_eq!(K::MIN_TURNSPEED.0, k("Constants.MIN_TURNSPEED"));
    assert_eq!(K::CONST_WATER_HEIGHT.0, k("Constants.WATER_HEIGHT"));
    assert_eq!(K::POS_CORRECTION_STR.0, k("PhysTools.posCorrectionStr"));
    assert_eq!(K::SLOP.0, k("PhysTools.slop"));
}

#[test]
fn vec2_ops() {
    let d = need!();
    let mut bad = 0;
    let vv = &d["vec2"];
    for row in vv["magnitude"].as_array().unwrap() {
        check(
            "magnitude",
            &row.to_string(),
            v2(&row[0]).magnitude(),
            &row[1],
            &mut bad,
        );
    }
    for row in vv["sqr_magnitude"].as_array().unwrap() {
        check(
            "sqr",
            &row.to_string(),
            v2(&row[0]).sqr_magnitude(),
            &row[1],
            &mut bad,
        );
    }
    for row in vv["normalized"].as_array().unwrap() {
        check_v(
            "normalized",
            &row.to_string(),
            v2(&row[0]).normalized(),
            &row[1],
            &mut bad,
        );
    }
    for row in vv["normalized_safe"].as_array().unwrap() {
        check_v(
            "nsafe",
            &row.to_string(),
            v2(&row[0]).normalized_safe(),
            &row[1],
            &mut bad,
        );
    }
    for row in vv["dot"].as_array().unwrap() {
        check(
            "dot",
            &row.to_string(),
            v2(&row[0]).dot(v2(&row[1])),
            &row[2],
            &mut bad,
        );
    }
    for row in vv["cross"].as_array().unwrap() {
        check(
            "cross",
            &row.to_string(),
            v2(&row[0]).cross(v2(&row[1])),
            &row[2],
            &mut bad,
        );
    }
    for row in vv["complex_mul"].as_array().unwrap() {
        let g = v2(&row[0]).complex_mul(v2(&row[1]));
        check_v("cmul", &row.to_string(), g, &row[2], &mut bad);
    }
    for row in vv["distance"].as_array().unwrap() {
        check(
            "dist",
            &row.to_string(),
            v2(&row[0]).distance(v2(&row[1])),
            &row[2],
            &mut bad,
        );
    }
    for row in vv["normalized_vector_angle"].as_array().unwrap() {
        let g = v2(&row[0]).normalized_vector_angle();
        check("nva", &row.to_string(), g, &row[1], &mut bad);
    }
    for row in vv["from_angle"].as_array().unwrap() {
        check_v(
            "from_angle",
            &row.to_string(),
            Vec2::from_angle(f(&row[0])),
            &row[1],
            &mut bad,
        );
    }
    for row in vv["slerp"].as_array().unwrap() {
        let g = v2(&row[0]).slerp(v2(&row[1]), f(&row[2]));
        check_v("slerp", &row.to_string(), g, &row[3], &mut bad);
    }
    for row in vv["nlerp"].as_array().unwrap() {
        let g = v2(&row[0]).nlerp(v2(&row[1]), f(&row[2]));
        check_v("nlerp", &row.to_string(), g, &row[3], &mut bad);
    }
    for row in vv["approach_normalized_vector"].as_array().unwrap() {
        let g = v2(&row[0]).approach_normalized(v2(&row[1]), f(&row[2]));
        check_v("approach", &row.to_string(), g, &row[3], &mut bad);
    }
    for row in vv["tools_angle2"].as_array().unwrap() {
        let g = Vec2::angle2(v2(&row[0]), v2(&row[1]));
        check("angle2", &row.to_string(), g, &row[2], &mut bad);
    }
    for row in vv["tools_reflect"].as_array().unwrap() {
        let g = v2(&row[0]).reflect(v2(&row[1]));
        check_v("reflect", &row.to_string(), g, &row[2], &mut bad);
    }
    assert_eq!(bad, 0);
}

#[test]
fn rng_sequences() {
    let d = need!();
    assert_eq!(
        Rng::default().state as i64,
        i(&d["rng"]["initial_state_after_static_ctor"])
    );
    for s in d["rng"]["updater_sequences"].as_array().unwrap() {
        let mut r = Rng::default();
        r.init_seed(i(&s["seed"]) as u32);
        assert_eq!(r.state as i64, i(&s["state_after_init"]));
        for c in s["calls"].as_array().unwrap() {
            let name = c[0].as_str().unwrap();
            match name {
                "RandomInt" => {
                    assert_eq!(r.state as i64, i(&c[3]), "{c}");
                    let v = r.random_int(i(&c[1]) as i32, i(&c[2]) as i32);
                    assert_eq!(v as i64, i(&c[4]), "{c}");
                    assert_eq!(r.state as i64, i(&c[5]), "{c}");
                }
                "RandomFix" => {
                    assert_eq!(r.state as i64, i(&c[3]), "{c}");
                    let v = r.random_fix(f(&c[1]), f(&c[2]));
                    assert_eq!(v.0, i(&c[4]), "{c}");
                    assert_eq!(r.state as i64, i(&c[5]), "{c}");
                }
                "RandomUnitVector" => {
                    assert_eq!(r.state as i64, i(&c[1]), "{c}");
                    assert_eq!(r.random_unit_vector(), v2(&c[2]), "{c}");
                    assert_eq!(r.state as i64, i(&c[3]), "{c}");
                }
                "RandomInsideUnitCircle" => {
                    assert_eq!(r.state as i64, i(&c[1]), "{c}");
                    assert_eq!(r.random_inside_unit_circle(), v2(&c[2]), "{c}");
                    assert_eq!(r.state as i64, i(&c[3]), "{c}");
                }
                _ => panic!("unknown call {name}"),
            }
        }
    }
}

#[test]
fn input_tables() {
    let d = need!();
    let t = bopl_sim::input::input_vector_table();
    for (b, e) in d["input"]["input_vector_table"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        assert_eq!(t[b], v2(e), "byte {b}");
    }
    for row in d["input"]["joystick_angle_byte"].as_array().unwrap() {
        let got = bopl_sim::input::angle_byte_from_vector(v2(&row[0]));
        assert_eq!(got as i64, i(&row[1]), "{row}");
    }
}
