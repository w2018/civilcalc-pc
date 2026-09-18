/**
 * 更新检查（组 14）—— 2 个命令。
 *
 * 见 docs/08-IPC契约.md §3 组 14。
 *
 * ## 🔴 「没更新」与「没查成」是两件事
 *
 * `updateCheck()` **不会因为连不上 GitHub 而抛错**（「检查更新」失败不该
 * 弹错误框打断用户）。但它也**不再**把失败伪装成「已是最新」：
 *
 * | `checkOk` | `degraded` | 含义 |
 * |---|---|---|
 * | `false` | — | **没查成**，`hasUpdate: false` 不可信，看 `failReason` |
 * | `true` | `true` | 主路径失败、走了兜底，只有版本号可信 |
 * | `true` | `false` | 真查到了，`hasUpdate` 可信 |
 *
 * 为什么重要：GitHub 的**匿名** API 是 60 次/小时/IP，撞上限流时
 * 旧实现会显示「没有可用的更新」—— 用户以为功能坏了，我们也没法自证。
 *
 * ## 桌面端不自下载
 *
 * PC 端安装走 NSIS，由用户手动完成。
 * `updateOpenDownload` 只负责用系统浏览器打开下载地址。
 */

import { invoke } from './invoke'

/**
 * 检查失败的原因码。
 *
 * ⚠️ 与后端 `UpdateFailReason` 逐字一致，由契约测试
 * `enum_values_match_ts` 钉住 —— 改这里必须同时改 Rust。
 */
export type UpdateFailReason =
  | 'offline'
  | 'rateLimited'
  | 'notFound'
  | 'httpError'
  | 'parseError'

/** 一次更新检查的结果 */
export interface UpdateInfo {
  hasUpdate: boolean
  latestVersion: string
  currentVersion: string
  releaseNotes: string
  /** 选中的安装包下载地址（`.exe` / `.msi` 优先），没有则 `null` */
  downloadUrl: string | null
  assetSize: number
  /** 形如 `"sha256:..."`，无则 `null` */
  assetDigest: string | null

  /** 这次检查**到底查成没查成**。`false` 时 `hasUpdate` 不代表「已是最新」 */
  checkOk: boolean
  /**
   * **主路径**（GitHub API）的失败原因。
   *
   * - `checkOk === false` → 彻底失败的原因；
   * - `checkOk === true && degraded === true` → 说明为什么降级；
   * - 主路径成功 → `null`。
   */
  failReason: UpdateFailReason | null
  /** 结果来自兜底路径：只有版本号准，安装包大小 / 摘要 / 直链都没有 */
  degraded: boolean
}

/**
 * 检查更新。
 *
 * 后端超时 30s，界面要给出 loading 态。
 * **不会因为网络问题抛错** —— 失败看 `checkOk` / `failReason`。
 */
export function updateCheck(): Promise<UpdateInfo> {
  return invoke<UpdateInfo>('update_check')
}

/**
 * 用系统默认浏览器打开下载地址。
 *
 * 只接受 `http` / `https`（其它 scheme 会被后端拒绝）。
 */
export function updateOpenDownload(url: string): Promise<void> {
  return invoke<void>('update_open_download', { url })
}

/** 字节数 → 人类可读（与 Rust `human_size` 同口径） */
export function humanSize(bytes: number): string {
  const KB = 1024
  const MB = KB * 1024
  if (bytes < KB) return `${Math.max(0, Math.round(bytes))} B`
  if (bytes < MB) return `${(bytes / KB).toFixed(1)} KB`
  return `${(bytes / MB).toFixed(1)} MB`
}
