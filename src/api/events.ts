/**
 * Tauri 事件订阅封装（组 13：导出进度 `export://progress`）。
 *
 * 规范（docs/05 §3.1.2）：组件不直接调 `@tauri-apps/api`，统一走 `src/api`。
 *
 * 返回 `unlisten`，调用方在 `onUnmounted` 里务必调用，**否则会泄漏监听器**。
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export function onTauriEvent<T>(
  event: string,
  cb: (payload: T) => void,
): Promise<UnlistenFn> {
  return listen<T>(event, (e) => cb(e.payload))
}
