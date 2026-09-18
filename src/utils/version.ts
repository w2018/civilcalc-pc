/**
 * 版本号（单一来源）。
 *
 * ## 为什么从 `package.json` 取而不是硬编码
 *
 * 侧栏底部原本写死 `v0.1.0`，版本升到 1.0.0 后它还在显示旧值 ——
 * 用户报版本号时会与「关于」页对不上。
 *
 * 版本号有三处（`Cargo.toml` / `tauri.conf.json` / `package.json`），
 * 由 `scripts/bump-version.sh` 一起改，契约测试 `versions_are_in_sync`
 * 保证三者一致。前端直接用 `package.json` 的值即可，**不要再写死**。
 *
 * 命名导入会被 Vite 摇树，不会把整个 `package.json` 打进产物。
 */
import { version } from '../../package.json'

/** 应用版本（形如 `1.0.0`，不带 `v` 前缀） */
export const APP_VERSION: string = version

/** 带 `v` 前缀的展示用版本 */
export const APP_VERSION_TAG = `v${APP_VERSION}`
