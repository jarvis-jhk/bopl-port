//! End-to-end import of the player's copy of the game (skipped when
//! `BOPL_GAME_DIR` is not set). Output goes to Cargo's target tmp dir.

use std::path::PathBuf;

use bopl_import::format::DetShapeKind;
use bopl_import::{GameData, Options, import};

#[test]
fn imports_the_game() {
    let Some(game) = std::env::var_os("BOPL_GAME_DIR").map(PathBuf::from) else {
        eprintln!("skipped: BOPL_GAME_DIR not set");
        return;
    };
    if !bopl_import::paths::is_bopl_dir(&game) {
        eprintln!("skipped: {} is not a Bopl Battle install", game.display());
        return;
    }
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("import-test");
    let opts = Options {
        game_dir: game.clone(),
        out_dir: out.clone(),
        force: true,
    };
    let r = import(&opts).unwrap();
    assert!(!r.up_to_date);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.counts["levels"], 124);
    assert_eq!(r.counts["scenes"], 6);
    // A second run without --force is a no-op.
    let again = import(&Options {
        force: false,
        ..opts
    })
    .unwrap();
    assert!(again.up_to_date);
    assert!(bopl_import::is_up_to_date(&game, &out));

    let data = GameData::load(&out).unwrap();
    assert_eq!(data.level_indices(), (6..=129).collect::<Vec<_>>());
    assert_eq!(data.abilities.full.len(), 31);
    assert!(data.abilities.full.iter().skip(1).all(|a| a.kind.is_some()));
    let player = data.prefab(data.player.prefab.as_deref().unwrap()).unwrap();
    assert!(player.objects[0].component("SlimeController").is_some());
    for e in &data.abilities.full {
        let p = data.prefab(e.prefab.as_deref().unwrap()).unwrap();
        assert_eq!(p.objects[0].name, e.name);
    }
    let lv = data.level(6).unwrap();
    assert_eq!(lv.team_spawns.len(), 4);
    assert!(!lv.platforms.is_empty());
    for &i in &lv.platforms {
        let o = &lv.objects[i];
        let (fix, shape) = (o.fix.as_ref().unwrap(), o.shape.as_ref().unwrap());
        assert!(matches!(shape.kind, DetShapeKind::RoundedRect { .. }));
        // The FixTransform agrees with Unity's float transform.
        let x = fix.position[0] as f64 / 4_294_967_296.0;
        assert!((x - o.world.trs.position[0] as f64).abs() < 1e-3);
    }
    assert!(data.localization.strings["en"].len() > 50);
    assert!(
        data.texture_path(data.textures.keys().next().unwrap())
            .unwrap()
            .is_file()
    );
}
