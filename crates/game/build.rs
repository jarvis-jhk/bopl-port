//! Adds the project-local SDL2 link directory (see docs/platform.md):
//! Linux: `tools/lib` (a symlink to the system's `libSDL2-2.0.so.0`, so no
//! -dev package is needed); Windows (cross, `scripts/build-windows.sh`): the
//! SDL2 mingw development release under `tools/windows/sdl2` plus the
//! libgcc/libgcc_eh shims (llvm-mingw's compiler-rt/libunwind) in
//! `tools/windows/gcc-shim`, both from `scripts/fetch-windows-toolchain.sh`.
//! `BOPL_SDL2_LIB_DIR` overrides the SDL2 directory.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=BOPL_SDL2_LIB_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let windows = std::env::var("TARGET")
        .unwrap_or_default()
        .contains("windows");
    let mut dirs = vec![root.join(if windows {
        "tools/windows/sdl2/x86_64-w64-mingw32/lib"
    } else {
        "tools/lib"
    })];
    if let Ok(dir) = std::env::var("BOPL_SDL2_LIB_DIR") {
        dirs[0] = dir.into();
    }
    if windows {
        dirs.push(root.join("tools/windows/gcc-shim"));
    }
    for dir in dirs.into_iter().filter(|d| d.exists()) {
        println!(
            "cargo:rustc-link-search=native={}",
            dir.canonicalize().unwrap().display()
        );
    }
}
