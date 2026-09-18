#!/usr/bin/env bash
# =============================================================================
#  dev-env.sh -- Run a command inside the MSVC x64 toolchain environment.
#
#  For Git Bash on Windows. See also scripts/dev-env.bat for cmd/PowerShell.
#
#  Why this exists (two separate Windows + Git Bash problems):
#
#   1) link.exe shadowing
#      GNU coreutils' link.exe (C:\Program Files\coreutils\bin\link.exe)
#      shadows MSVC's link.exe in PATH. Cargo then invokes the wrong tool:
#          link: extra operand '...rcgu.o'
#
#   2) vcvars64.bat is not always usable
#      It shells out to reg.exe to discover the Windows SDK. In restricted
#      environments reg.exe may be blocked, leaving PATH set but LIB unset:
#          LINK : fatal error LNK1181: cannot open input file 'kernel32.lib'
#
#  This script sets PATH / LIB / INCLUDE directly from the filesystem
#  (no reg.exe, no batch quoting), picking the highest installed versions.
#
#  Usage:
#     ./scripts/dev-env.sh cargo check -p civilcalc-core
#     ./scripts/dev-env.sh cargo test --workspace
#     ./scripts/dev-env.sh cargo build -p civilcalc-pc --release
#     DEVENV_VERBOSE=1 ./scripts/dev-env.sh cargo --version
#
#  Note:
#     cargo 会被自动追加 --offline（见文件末尾说明）。
#     需要联网时用 DEVENV_ONLINE=1 前缀。
# =============================================================================
set -euo pipefail

log() { [ -n "${DEVENV_VERBOSE:-}" ] && echo "[dev-env] $*" >&2 || true; }
die() { echo "[ERROR] $*" >&2; exit 1; }

# ------------------------------------------------------------------ 1. VS path
find_vs() {
  local candidates=(
    "${VSINSTALLDIR:-}"
    "/d/ProgramData/Microsoft/MicrosoftVisualStudio/18/BuildTools"
    "/c/Program Files/Microsoft Visual Studio/2022/BuildTools"
    "/c/Program Files/Microsoft Visual Studio/2022/Community"
    "/c/Program Files/Microsoft Visual Studio/2022/Professional"
    "/c/Program Files/Microsoft Visual Studio/2022/Enterprise"
    "/d/Program Files/Microsoft Visual Studio/2022/BuildTools"
  )
  local c
  for c in "${candidates[@]}"; do
    [ -n "$c" ] && [ -d "$c/VC/Tools/MSVC" ] && { echo "$c"; return 0; }
  done
  return 1
}

VS_PATH="$(find_vs)" || die "Visual Studio / Build Tools not found.
        Install \"Desktop development with C++\":
        https://visualstudio.microsoft.com/visual-cpp-build-tools/"
log "VS = $VS_PATH"

# ------------------------------------------------------- 2. MSVC toolset x64
MSVC_ROOT="$VS_PATH/VC/Tools/MSVC"
MSVC_VER="$(ls -1 "$MSVC_ROOT" 2>/dev/null | sort -V | tail -1)"
[ -n "$MSVC_VER" ] || die "No MSVC toolset under: $MSVC_ROOT"

VC_BIN="$MSVC_ROOT/$MSVC_VER/bin/Hostx64/x64"
VC_LIB="$MSVC_ROOT/$MSVC_VER/lib/x64"
VC_INC="$MSVC_ROOT/$MSVC_VER/include"

[ -x "$VC_BIN/link.exe" ] || die "link.exe not found at: $VC_BIN"
log "MSVC = $MSVC_VER"

# ---------------------------------------------------------- 3. Windows SDK 10
SDK_ROOT=""
for c in \
  "/c/Program Files (x86)/Windows Kits/10" \
  "/d/Program Files (x86)/Windows Kits/10" \
  "/c/Program Files/Windows Kits/10"
do
  [ -d "$c/Lib" ] && { SDK_ROOT="$c"; break; }
done
[ -n "$SDK_ROOT" ] || die "Windows SDK not found. Install the \"Windows 10/11 SDK\" component."

SDK_VER="$(ls -1 "$SDK_ROOT/Lib" 2>/dev/null | sort -V | tail -1)"
[ -n "$SDK_VER" ] || die "No Windows SDK version under: $SDK_ROOT/Lib"

SDK_LIB="$SDK_ROOT/Lib/$SDK_VER"
SDK_INC="$SDK_ROOT/Include/$SDK_VER"

[ -f "$SDK_LIB/um/x64/kernel32.lib" ] || die "kernel32.lib not found at: $SDK_LIB/um/x64"
log "SDK  = $SDK_VER"

# --------------------------------------------------------------------- 4. env
# PATH first, so MSVC's link.exe wins over coreutils' link.exe
export PATH="$VC_BIN:$PATH"

# LIB / INCLUDE must be Windows-style paths: link.exe does not understand
# Git Bash paths like /d/ProgramData/...
w() { cygpath -w "$1" 2>/dev/null || echo "$1"; }

export LIB="$(w "$VC_LIB");$(w "$SDK_LIB/ucrt/x64");$(w "$SDK_LIB/um/x64")"
export INCLUDE="$(w "$VC_INC");$(w "$SDK_INC/ucrt");$(w "$SDK_INC/um");$(w "$SDK_INC/shared")"

log "LINK  = $VC_BIN/link.exe"
log "LIB   = $LIB"

# 关掉增量编译。
#
# 原因（本机实测）：cargo 把 .rmeta/.rlib 复制进 target/debug/incremental/ 时，
# 会**间歇性**报「拒绝访问 (os error 5)」——
#     warning: error copying object file ... to incremental directory ...: 拒绝访问
#     error: failed to write `target\debug\.fingerprint\...\lib-xxx.json`: 拒绝访问
# 同一目录上并发写多个小文件时必现（杀软实时扫描持有句柄 + Windows 删除语义），
# 单 crate 单独编译则稳定通过。
#
# 关掉增量编译后：
#   - 不再写 incremental/ 目录，彻底绕开该问题
#   - 首次全量编译略慢，但稳定；CI/发布构建本来也是全量
# 需要增量编译时：DEVENV_INCREMENTAL=1
if [ -z "${DEVENV_INCREMENTAL:-}" ]; then
  export CARGO_INCREMENTAL=0
fi

# --------------------------------------------------------------------- 5. run
if [ $# -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

# cargo 默认补 --offline（**仅构建类子命令**）。
#
# 原因（实测）：~/.cargo/config.toml 用镜像源时，索引更新与 crate 下载可能
# 长时间无输出 —— 看起来像卡死，实际是在慢慢下载。而 Cargo.lock 与本地
# registry 缓存齐全时，build/check/test/clippy 根本不需要联网。
#
# 但 fetch / add / update / install / search / publish 本质就是联网操作，
# 加 --offline 只会得到 "attempting to make an HTTP request" 错误。
#
# 需要联网时（或想显式联网构建）：
#     DEVENV_ONLINE=1 ./scripts/dev-env.sh cargo add serde
OFFLINE_SUBCMDS="build check test bench run clippy rustc doc fix"
if [ "${1:-}" = "cargo" ] && [ -z "${DEVENV_ONLINE:-}" ]; then
  sub="${2:-}"
  case " $OFFLINE_SUBCMDS " in
    *" $sub "*)
      shift
      log "cargo $sub 追加 --offline（DEVENV_ONLINE=1 可关闭）"
      exec cargo --offline "$@"
      ;;
  esac
fi

exec "$@"
