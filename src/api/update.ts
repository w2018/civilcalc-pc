/**
 * 更新检查（组 14）—— 2 个命令。
 *
 * 见 docs/08-IPC契约.md §3 组 14。
 *
 * ## 🔴 网络失败是**静默**的
 *
 * `updateCheck()` **不会因为连不上 GitHub 而抛错** —— 它返回
 * `hasUpdate: false`（`latestVersion` = 当前版本）。
 * 所以「检查更新」按钮不需要 `catch` 网络错误，但也**无法**用它判断
 * 「到底是最新版还是没连上」。要区分只能另想办法（当前不做）。
 *
 * ## 桌面端不自下载
 *
 * PC 端安装走 NSIS / MSI，由用户手动完成。
 * `updateOpenDownload` 只负责用系统浏览器打开下载地址。
 */

import { invoke } from './invoke'

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
}

/**
 * 检查更新。
 *
 * ⚠️ 网络失败 / 无 releases / 解析异常 → 一律返回「当前已是最新」。
 * 后端超时 30s，界面要给出 loading 态。
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
