# Source this (after env.sh) to cross-compile for Windows with the
# project-local llvm-mingw from scripts/fetch-windows-toolchain.sh:
#   source env.sh && source scripts/windows-env.sh
#   cargo build --release --target x86_64-pc-windows-gnu -p bopl-port
# llvm-mingw's bin dir goes on PATH because rustc calls
# x86_64-w64-mingw32-dlltool for raw-dylib imports (windows-rs crates).
# The gcc-shim directory (libgcc/libgcc_eh mapped onto llvm-mingw's
# compiler-rt/libunwind) goes on the link path of every Windows binary
# (the game's build.rs adds it too; this covers sim-trace, tests, xcheck).
# Windows builds use their own target dir (target/windows) so they never
# wait for, or invalidate, the Linux builds in target/.
_W="$BOPL_ROOT/tools/windows"
if [ ! -x "$_W/llvm-mingw/bin/x86_64-w64-mingw32-clang" ]; then
    echo "Windows toolchain missing: run scripts/fetch-windows-toolchain.sh" >&2
    unset _W
    return 1 2>/dev/null || exit 1
fi
export PATH="$_W/llvm-mingw/bin:$PATH"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$_W/llvm-mingw/bin/x86_64-w64-mingw32-clang"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L native=$_W/gcc-shim"
export CC_x86_64_pc_windows_gnu="$_W/llvm-mingw/bin/x86_64-w64-mingw32-clang"
export AR_x86_64_pc_windows_gnu="$_W/llvm-mingw/bin/llvm-ar"
export BOPL_WIN_TARGET_DIR="${BOPL_WIN_TARGET_DIR:-$BOPL_ROOT/target/windows}"
unset _W
