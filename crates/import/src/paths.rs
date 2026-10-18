//! Where things live: the user data dir and the original game's install.
//!
//! - Data dir: `$BOPL_PORT_DATA_DIR`, else the platform's per-user data
//!   dir (`$XDG_DATA_HOME/bopl-port` or `~/.local/share/bopl-port` on
//!   Linux, `%APPDATA%\bopl-port` on Windows, `~/Library/Application
//!   Support/bopl-port` on macOS). The import lives in
//!   `<data dir>/import-v<N>` ([`import_dir`]).
//! - Bopl Battle install, first match: `--game DIR`, `$BOPL_GAME_DIR`,
//!   `$WARPZONE_DATA_BOPL` (set by the Warpzone launcher for a data
//!   requirement with id `bopl`), then every Steam library
//!   (`steamapps/libraryfolders.vdf`) of every Steam root found:
//!   `steamapps/common/Bopl Battle`.

use std::path::{Path, PathBuf};

const APP_DIR: &str = "bopl-port";
/// Steam app id of Bopl Battle.
const BOPL_APP_ID: &str = "1686940";
/// Install folder name under `steamapps/common`.
const INSTALL_DIR: &str = "Bopl Battle";

/// The import dir of the current format inside the user data dir.
pub fn import_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join(crate::versioned_dir_name()))
}

/// The user data dir (not created here).
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("BOPL_PORT_DATA_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    platform_data_dir().map(|d| d.join(APP_DIR))
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Option<PathBuf> {
    home().map(|h| h.join("Library/Application Support"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_data_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(x));
    }
    home().map(|h| h.join(".local/share"))
}

fn home() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Whether `dir` looks like a Bopl Battle install (or its `BoplBattle_Data` dir).
pub fn is_bopl_dir(dir: &Path) -> bool {
    dir.join("BoplBattle_Data")
        .join("globalgamemanagers")
        .is_file()
        || dir.join("globalgamemanagers").is_file() && dir.join("level0").is_file()
}

/// Where an install was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    /// The game directory.
    pub dir: PathBuf,
    /// Human-readable origin ("--game", "BOPL_GAME_DIR", "Steam library ...").
    pub source: String,
}

/// Finds the Bopl Battle install. `cli` is `--game DIR`. On failure, returns the
/// list of places that were tried.
pub fn find_install(cli: Option<&Path>) -> Result<Install, Vec<String>> {
    let mut tried = Vec::new();
    let mut check = |dir: PathBuf, source: String| -> Option<Install> {
        if is_bopl_dir(&dir) {
            Some(Install { dir, source })
        } else {
            tried.push(format!("{} ({source})", dir.display()));
            None
        }
    };
    if let Some(d) = cli
        && let Some(i) = check(d.to_path_buf(), "--game".into())
    {
        return Ok(i);
    }
    for var in ["BOPL_GAME_DIR", "WARPZONE_DATA_BOPL"] {
        if let Some(d) = std::env::var_os(var).filter(|d| !d.is_empty())
            && let Some(i) = check(PathBuf::from(d), var.into())
        {
            return Ok(i);
        }
    }
    for lib in steam_libraries() {
        let dir = lib.join("steamapps").join("common").join(INSTALL_DIR);
        if let Some(i) = check(dir, format!("Steam library {}", lib.display())) {
            return Ok(i);
        }
    }
    Err(tried)
}

/// Steam installation roots that exist on this machine.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Some(p) = windows_registry_steam_path() {
            roots.push(p);
        }
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = std::env::var_os(var) {
                roots.push(PathBuf::from(p).join("Steam"));
            }
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    } else if let Some(h) = home() {
        for rel in [
            ".steam/steam",
            ".steam/root",
            ".local/share/Steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            ".var/app/com.valvesoftware.Steam/data/Steam",
            "snap/steam/common/.local/share/Steam",
            "Library/Application Support/Steam",
        ] {
            roots.push(h.join(rel));
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for r in roots {
        if !r.join("steamapps").is_dir() {
            continue;
        }
        let canon = std::fs::canonicalize(&r).unwrap_or(r);
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
    out
}

/// All Steam library folders (each holding `steamapps/`), the roots first.
/// Libraries whose manifest lists Bopl Battle come before the others.
pub fn steam_libraries() -> Vec<PathBuf> {
    let mut libs: Vec<(bool, PathBuf)> = Vec::new();
    for root in steam_roots() {
        let mut push = |p: PathBuf, has: bool| {
            let canon = std::fs::canonicalize(&p).unwrap_or(p);
            if let Some(e) = libs.iter_mut().find(|(_, q)| *q == canon) {
                e.0 |= has;
            } else {
                libs.push((has, canon));
            }
        };
        push(root.clone(), false);
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(&vdf) {
            for (path, apps) in parse_library_folders(&text) {
                push(PathBuf::from(path), apps.iter().any(|a| a == BOPL_APP_ID));
            }
        }
    }
    libs.sort_by_key(|(has, _)| !*has);
    libs.into_iter().map(|(_, p)| p).collect()
}

#[cfg(windows)]
fn windows_registry_steam_path() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    for (key, value) in [
        (r"HKCU\Software\Valve\Steam", "SteamPath"),
        (r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (r"HKLM\SOFTWARE\Valve\Steam", "InstallPath"),
    ] {
        let out = std::process::Command::new("reg")
            .args(["query", key, "/v", value])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix(value) {
                let rest = rest.trim_start();
                if let Some(v) = rest.strip_prefix("REG_SZ") {
                    let p = v.trim().replace('/', "\\");
                    if !p.is_empty() {
                        return Some(PathBuf::from(p));
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn windows_registry_steam_path() -> Option<PathBuf> {
    None
}

/// Parses Steam's `libraryfolders.vdf`: `(path, app ids)` per library.
/// Handles both the current format (`"0" { "path" "..." "apps" {...} }`) and
/// the old one (`"1" "D:\\SteamLibrary"`).
pub fn parse_library_folders(text: &str) -> Vec<(String, Vec<String>)> {
    // Tokenise quoted strings and braces.
    let mut toks: Vec<String> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' => toks.push(c.to_string()),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n);
                            }
                        }
                        '"' => break,
                        _ => s.push(c),
                    }
                }
                toks.push(format!("\"{s}"));
            }
            _ => {}
        }
    }
    let unq = |t: &str| t.strip_prefix('"').map(str::to_owned);
    let mut out = Vec::new();
    // Walk: key value | key { ... }, tracking depth.
    let mut i = 0;
    let mut depth = 0;
    let mut current: Option<(String, Vec<String>)> = None;
    let mut in_apps = false;
    while i < toks.len() {
        let t = &toks[i];
        if t == "{" {
            depth += 1;
            i += 1;
            continue;
        }
        if t == "}" {
            if in_apps && depth == 3 {
                in_apps = false;
            }
            if depth == 2
                && let Some(c) = current.take()
            {
                out.push(c);
            }
            depth -= 1;
            i += 1;
            continue;
        }
        let key = unq(t).unwrap_or_default();
        let next = toks.get(i + 1).map(String::as_str);
        match next {
            Some("{") => {
                if depth == 1 {
                    current = Some((String::new(), Vec::new()));
                } else if depth == 2 && key.eq_ignore_ascii_case("apps") {
                    in_apps = true;
                }
                i += 1;
            }
            Some(v) if v.starts_with('"') => {
                let val = unq(v).unwrap_or_default();
                if depth == 1 && key.chars().all(|c| c.is_ascii_digit()) {
                    // Old format: "1" "path".
                    out.push((val, Vec::new()));
                } else if depth == 2 && key.eq_ignore_ascii_case("path") {
                    if let Some(c) = current.as_mut() {
                        c.0 = val;
                    }
                } else if depth == 3
                    && in_apps
                    && let Some(c) = current.as_mut()
                {
                    c.1.push(key);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    out.retain(|(p, _)| !p.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_folders_new_format() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/u/.local/share/Steam"
		"label"		""
		"apps"
		{
			"228980"		"493211355"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps"
		{
			"1686940"		"1189430131"
		}
	}
}
"#;
        let libs = parse_library_folders(vdf);
        assert_eq!(libs.len(), 2);
        assert_eq!(libs[0].0, "/home/u/.local/share/Steam");
        assert_eq!(libs[0].1, vec!["228980"]);
        assert_eq!(libs[1].0, r"D:\SteamLibrary");
        assert_eq!(libs[1].1, vec!["1686940"]);
    }

    #[test]
    fn library_folders_old_format() {
        let vdf = r#""LibraryFolders" { "TimeNextStatsReport" "123" "1" "E:\\Games\\Steam" }"#;
        let libs = parse_library_folders(vdf);
        assert_eq!(libs, vec![(r"E:\Games\Steam".to_string(), vec![])]);
    }

    #[test]
    fn data_dir_env_override() {
        // Only checks the override logic, without touching the environment
        // of other tests: the variable is set by env.sh in development.
        if let Some(d) = std::env::var_os("BOPL_PORT_DATA_DIR") {
            assert_eq!(data_dir(), Some(PathBuf::from(d)));
        } else {
            assert!(data_dir().is_some_and(|d| d.ends_with(APP_DIR)));
        }
    }
}
