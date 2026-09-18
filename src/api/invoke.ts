import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import type { CommandError } from '@/types/error'

/**
 * 统一 invoke 封装 —— **前端唯一的 IPC 出口**。
 *
 * 规范（见 docs/05-项目开发方案.md §3.1.2）：
 * - 视图与组件**不得**直接调用 `@tauri-apps/api`
 * - 错误统一归一化为 `CommandError` 可判别联合
 */
export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args)
  } catch (e) {
    throw normalizeError(e)
  }
}

/**
 * 把 Tauri 抛出的错误归一化为 `CommandError`。
 *
 * Rust 侧 `CommandError` 用 `#[serde(tag = "kind")]` 序列化，
 * 因此正常情况下会直接得到带 `kind` 的对象；此处只做兜底。
 */
export function normalizeError(e: unknown): CommandError {
  if (e && typeof e === 'object' && 'kind' in e) {
    return e as CommandError
  }
  return { kind: 'unknown', message: typeof e === 'string' ? e : String(e) }
}
