//! CLI: `bopl-import [--game DIR] [--out DIR | --dev] [--force]`.
//!
//! Defaults: the game is found like the port finds it (`--game`,
//! `$BOPL_GAME_DIR`, `$WARPZONE_DATA_BOPL`, Steam libraries); the output is
//! `<data dir>/import-v<N>` (`$BOPL_PORT_DATA_DIR`, else the platform's user
//! data dir). `--dev` writes `cache/import` under `$BOPL_ROOT` instead.

use std::path::PathBuf;
use std::process::ExitCode;

use bopl_import::paths;

fn usage() -> ExitCode {
    eprintln!("usage: bopl-import [--game DIR] [--out DIR | --dev] [--force]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut game: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut force = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--game" => match args.next() {
                Some(g) => game = Some(g.into()),
                None => return usage(),
            },
            "--out" => match args.next() {
                Some(o) => out = Some(o.into()),
                None => return usage(),
            },
            "--dev" => {
                let root = std::env::var_os("BOPL_ROOT")
                    .map(PathBuf::from)
                    .unwrap_or_default();
                out = Some(root.join("cache/import"));
            }
            "--force" => force = true,
            "-h" | "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            _ => return usage(),
        }
    }
    let install = match paths::find_install(game.as_deref()) {
        Ok(i) => i,
        Err(tried) => {
            eprintln!("Bopl Battle not found; tried:");
            for t in tried {
                eprintln!("  {t}");
            }
            eprintln!("pass --game DIR or set BOPL_GAME_DIR");
            return ExitCode::FAILURE;
        }
    };
    let Some(out) = out.or_else(paths::import_dir) else {
        eprintln!("no user data dir: pass --out DIR or set BOPL_PORT_DATA_DIR");
        return ExitCode::FAILURE;
    };
    let opts = bopl_import::Options {
        game_dir: install.dir,
        out_dir: out.clone(),
        force,
    };
    match bopl_import::import(&opts) {
        Ok(r) if r.up_to_date => {
            println!("{} is up to date (use --force to re-import)", out.display());
            ExitCode::SUCCESS
        }
        Ok(r) => {
            for w in &r.warnings {
                eprintln!("warning: {w}");
            }
            let counts: Vec<String> = r.counts.iter().map(|(k, v)| format!("{v} {k}")).collect();
            println!(
                "imported into {} in {:.2}s: {}",
                out.display(),
                r.seconds,
                counts.join(", ")
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("import failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}
