/**
 * 系统打开器封装（组 13：导出后「打开文件」）。
 *
 * 见 docs/08-IPC契约.md 组 13。
 *
 * - `report_reveal(path)`（后端命令）**定位到所在文件夹**
 * - `openFile(path)`（本文件）**用默认程序打开该文件**（如 Word 打开 .docx）
 *
 * 规范（docs/05 §3.1.2）：组件不直接调 `@tauri-apps/api`，统一走 `src/api`。
 */
import { openPath } from '@tauri-apps/plugin-opener'

/** 用系统默认程序打开文件（如 .docx → Word） */
export async function openFile(path: string): Promise<void> {
  await openPath(path)
}
