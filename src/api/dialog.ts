/**
 * 原生对话框封装（组 13：导出落盘用）。
 *
 * 见 docs/08-IPC契约.md 组 13 的 `chosenDirectory` / `chosenFile` 两档。
 *
 * ## 为什么放在 API 层
 *
 * 规范（docs/05 §3.1.2）：视图与组件**不得**直接调用 `@tauri-apps/api`，
 * 统一走 `src/api`。这样将来换 Web/WASM 后端时只改 API 层。
 *
 * ## `open` / `save` 的返回
 *
 * - `open({ directory: true })`：`string | string[] | null`
 * - `save(...)`：`string | null`
 * 这里都只取单个字符串，多选/取消统一归为 `null`。
 */
import { open, save } from '@tauri-apps/plugin-dialog'

/** 选一个目录（导出到「用户选的目录」档位） */
export async function pickDirectory(): Promise<string | null> {
  const r = await open({ directory: true, multiple: false })
  return typeof r === 'string' ? r : null
}

/** 选一个保存路径（导出到「用户选的完整文件」档位） */
export async function pickSaveDocx(defaultName: string): Promise<string | null> {
  const r = await save({
    defaultPath: defaultName,
    filters: [{ name: 'Word 文档', extensions: ['docx'] }],
  })
  return r ?? null
}

/** 支持的图片扩展名（与 `image_save_from_path` 能解码的一致） */
export const IMAGE_EXTENSIONS = ['jpg', 'jpeg', 'png', 'webp', 'bmp', 'gif'] as const

/**
 * 选**一张或多张**图片（附图入口）。
 *
 * 返回规范化后的绝对路径数组；取消 / 选到目录时返回 `[]`。
 *
 * ⚠️ 这只是「选文件」。落盘、EXIF 方向修正、缩放到 1024、JPEG q80、去重
 * 全部由 `image_save_from_path` 做 —— 前端**不要**自己读文件内容。
 */
export async function pickImageFiles(multiple = true): Promise<string[]> {
  const r = await open({
    multiple,
    directory: false,
    filters: [{ name: '图片', extensions: [...IMAGE_EXTENSIONS] }],
  })
  if (r === null) return []
  return Array.isArray(r) ? r : [r]
}

/** 选一个图片保存路径（查看器的「另存为」） */
export async function pickImageSavePath(defaultName: string): Promise<string | null> {
  const r = await save({
    defaultPath: defaultName,
    filters: [{ name: '图片', extensions: ['jpg', 'png'] }],
  })
  return r ?? null
}

/** 选一个 JSON 保存路径（LLM 配置导出） */
export async function pickJsonSavePath(defaultName: string): Promise<string | null> {
  const r = await save({
    defaultPath: defaultName,
    filters: [{ name: 'JSON', extensions: ['json'] }],
  })
  return r ?? null
}

/**
 * 选一个 JSON 文件（LLM 配置导入）。
 *
 * 返回的路径会被 Tauri 的 fs scope 自动放行 —— 所以拿它去
 * `readTextFile` 是安全的，**不要**改成手输路径。
 */
export async function pickJsonFile(): Promise<string | null> {
  const r = await open({
    multiple: false,
    directory: false,
    filters: [{ name: 'JSON', extensions: ['json'] }],
  })
  if (r === null) return null
  return Array.isArray(r) ? (r[0] ?? null) : r
}

/**
 * 选一个备份包（本地导入）。
 *
 * 备份包名形如 `civilcalc_backup_20260915_143005.tar.gz` ——
 * 过滤器的扩展名只写 `gz`（Tauri 的 `extensions` 不带点，
 * 写 `tar.gz` 匹配不上任何文件）。
 *
 * 不校验文件名：**用户可能改过名**，能不能读由 `backup_inspect` 判断。
 */
export async function pickBackupFile(): Promise<string | null> {
  const r = await open({
    multiple: false,
    directory: false,
    filters: [{ name: '备份包', extensions: ['gz', 'tgz', 'tar'] }],
  })
  if (r === null) return null
  return Array.isArray(r) ? (r[0] ?? null) : r
}
