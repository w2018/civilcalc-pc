/**
 * 本地文件 → WebView 可访问 URL 的转换。
 *
 * 这是 `@tauri-apps/api/core` 里**除 `invoke` 之外唯一**需要在前端用到的东西
 * （`convertFileSrc`）。按 §3.1.2「api/ 是唯一 IPC 出口」的规范，
 * 在这里包一层，避免组件直接 import `@tauri-apps/api`。
 *
 * ## 为什么需要转换
 *
 * WebView 出于安全无法直接读 `file://`；Tauri 提供了 `asset:` 协议
 * （Windows 上是 `http://asset.localhost`），需要把绝对路径转成该协议 URL。
 *
 * `tauri.conf.json` 的 CSP 已放行：
 *
 * ```
 * img-src 'self' data: asset: http://asset.localhost
 * ```
 *
 * 新增图片类需求时**不要**顺手放宽 `img-src` —— 先确认是否真的需要。
 */

import { convertFileSrc } from '@tauri-apps/api/core'
import { readTextFile as fsReadTextFile, writeFile, writeTextFile } from '@tauri-apps/plugin-fs'

/**
 * 绝对路径 → `asset:` URL。
 *
 * @param absolutePath 本机绝对路径（Windows 形如 `D:\...\background.jpg`）
 * @param cacheBust 追加一个每次都不同的查询参数，**绕开 WebView 缓存**。
 *   文件路径固定不变的资源必须开 —— 背景图就是（见 `stores/ui.ts` 的说明）。
 */
export function assetSrc(absolutePath: string, cacheBust = false): string {
  const url = convertFileSrc(absolutePath)
  // ⚠️ 加查询串是安全的：Tauri 的 asset 协议处理器取的是 `request.uri().path()`
  //    （不含 query），所以 `?v=` 不会让它找不到文件，但 WebView 会当成新 URL。
  return cacheBust ? `${url}?v=${Date.now()}` : url
}

/**
 * data URL → 字节数组。
 *
 * 只处理 `data:<mime>;base64,<data>` 形式；不带 `base64` 的
 * （如 `data:text/plain,abc`）按 URL 编码解。解析失败抛错。
 */
export function dataUrlToBytes(dataUrl: string): Uint8Array {
  const comma = dataUrl.indexOf(',')
  if (comma < 0) throw new Error('不是合法的 data URL')
  const meta = dataUrl.slice(0, comma)
  const body = dataUrl.slice(comma + 1)

  if (!/;base64$/i.test(meta)) {
    return new TextEncoder().encode(decodeURIComponent(body))
  }

  const bin = atob(body)
  const bytes = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i)
  return bytes
}

/**
 * 把 data URL 写到用户选定的路径（查看器「另存为」）。
 *
 * ## 为什么写在 `api/asset.ts` 而不是组件里
 *
 * 规范（docs/05 §3.1.2）：组件**不得**直接 import `@tauri-apps/*`，
 * 统一走 `src/api`。这样将来换实现只改一处。
 *
 * ⚠️ 目标路径必须来自 `dialog.save`（用户主动选的文件）——
 * Tauri 的 fs scope 只对用户选过的路径放行，硬编码路径会被拒。
 */
export async function saveDataUrlToFile(dataUrl: string, path: string): Promise<void> {
  await writeFile(path, dataUrlToBytes(dataUrl))
}

/**
 * 把文本写到用户选定的路径（配置导出）。
 *
 * ⚠️ 目标路径同样必须来自 `dialog.save`（fs scope 只放行用户选过的路径）。
 */
export async function saveTextToFile(text: string, path: string): Promise<void> {
  await writeTextFile(path, text)
}

/**
 * 读一个文本文件（配置导入）。
 *
 * ⚠️ 路径必须来自 `dialog.open` —— 直接传任意绝对路径会被 fs scope 拒绝。
 */
export async function readTextFile(path: string): Promise<string> {
  return fsReadTextFile(path)
}
