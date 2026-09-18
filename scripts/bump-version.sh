#!/usr/bin/env bash
# =============================================================================
#  bump-version.sh -- 版本号自增（三处一起改）
#
#  依据：项目约定「每次新增功能或修复 BUG，版本号自动 +1」
#        起始版本 v1.0.0，之后 v1.0.1 / v1.0.2 / v1.0.3 …
#
#  ## 为什么必须是脚本
#
#  版本号有**三处**，且契约测试 `versions_are_in_sync`
#  （`src-tauri/tests/contract.rs`）会断言三者一致 ——
#  手改很容易漏一处，而漏了之后 `cargo test` 才会红。
#  用脚本一次改三处，从源头避免。
#
#  ## 用法
#
#     ./scripts/bump-version.sh            # 补丁位 +1（最常用）
#     ./scripts/bump-version.sh 1.2.0      # 显式指定版本
#     ./scripts/bump-version.sh --show     # 只打印当前版本
#     ./scripts/bump-version.sh --minor    # 次版本 +1，补丁归零
#     ./scripts/bump-version.sh --major    # 主版本 +1，其余归零
#
#  ## 改完记得
#
#     ./scripts/dev-env.sh cargo test --workspace    # 契约测试会校验三处一致
# =============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TOML="$ROOT/Cargo.toml"
TAURI_CONF="$ROOT/src-tauri/tauri.conf.json"
PKG_JSON="$ROOT/package.json"

die() { echo "[ERROR] $*" >&2; exit 1; }

for f in "$CARGO_TOML" "$TAURI_CONF" "$PKG_JSON"; do
  [ -f "$f" ] || die "找不到 $f"
done

# ------------------------------------------------------------------ 读当前版本
# 只取 `[workspace.package]` 段里的 version —— 别处（依赖项）也有 `version =`
read_cargo_version() {
  awk '
    /^\[workspace\.package\]/ { in_section = 1; next }
    /^\[/ { in_section = 0 }
    in_section && /^version[[:space:]]*=/ {
      gsub(/.*=[[:space:]]*"/, ""); gsub(/".*/, ""); print; exit
    }
  ' "$CARGO_TOML"
}

CUR="$(read_cargo_version)"
[ -n "$CUR" ] || die "没能从 $CARGO_TOML 的 [workspace.package] 读出 version"

case "${1:-}" in
  --show|"")
    if [ "${1:-}" = "--show" ]; then echo "$CUR"; exit 0; fi
    ;;
esac

# ------------------------------------------------------------------ 算新版本
MAJOR="${CUR%%.*}"
REST="${CUR#*.}"
MINOR="${REST%%.*}"
PATCH="${REST#*.}"

for part in "$MAJOR" "$MINOR" "$PATCH"; do
  case "$part" in
    ''|*[!0-9]*) die "版本号格式不是 x.y.z：$CUR" ;;
  esac
done

case "${1:-}" in
  "")
    NEW="$MAJOR.$MINOR.$((PATCH + 1))"
    ;;
  --minor)
    NEW="$MAJOR.$((MINOR + 1)).0"
    ;;
  --major)
    NEW="$((MAJOR + 1)).0.0"
    ;;
  *)
    NEW="$1"
    case "$NEW" in
      *[!0-9.]*|.*|*.) die "版本号格式应为 x.y.z：$NEW" ;;
    esac
    case "$NEW" in
      *.*.*) ;;
      *) die "版本号要三段：$NEW" ;;
    esac
    ;;
esac

# ------------------------------------------------------------------ 写三处
# Cargo.toml：只改 [workspace.package] 段内的那一行
awk -v new="$NEW" '
  /^\[workspace\.package\]/ { in_section = 1; print; next }
  /^\[/ { in_section = 0 }
  {
    if (in_section && /^version[[:space:]]*=/) {
      print "version = \"" new "\""
    } else {
      print
    }
  }
' "$CARGO_TOML" > "$CARGO_TOML.tmp"
mv "$CARGO_TOML.tmp" "$CARGO_TOML"

# tauri.conf.json / package.json：顶层 "version": "x.y.z" 各只有一处
sed -i -E "s/(\"version\"[[:space:]]*:[[:space:]]*\")[^\"]+(\")/\1$NEW\2/" "$TAURI_CONF"
sed -i -E "s/(\"version\"[[:space:]]*:[[:space:]]*\")[^\"]+(\")/\1$NEW\2/" "$PKG_JSON"

# ------------------------------------------------------------------ 校验
A="$(read_cargo_version)"
B="$(sed -n -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' "$TAURI_CONF" | head -1)"
C="$(sed -n -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' "$PKG_JSON" | head -1)"

[ "$A" = "$NEW" ] || die "Cargo.toml 写入失败（得到 $A）"
[ "$B" = "$NEW" ] || die "tauri.conf.json 写入失败（得到 $B）"
[ "$C" = "$NEW" ] || die "package.json 写入失败（得到 $C）"

echo "版本号：$CUR → $NEW"
echo "  Cargo.toml [workspace.package]  = $A"
echo "  src-tauri/tauri.conf.json       = $B"
echo "  package.json                    = $C"
echo
echo "别忘了：./scripts/dev-env.sh cargo test --workspace   # 契约测试校验三处一致"
