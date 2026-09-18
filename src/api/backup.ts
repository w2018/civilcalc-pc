/**
 * 组 14：本地备份（3 个命令）。
 *
 * 见 docs/08-IPC契约.md §3 组 14。
 *
 * ## 两段式：先 inspect 再 import
 *
 * `backupInspect` 只解析清单（不解码图片字节），供二次确认弹窗展示
 * 「这个包里有什么」。用户确认后才调 `backupLocalImport`。
 * 好处是确认弹窗里的数字**来自包本身**，不是估算。
 *
 * ## 🔴 两种导入口径
 *
 * | mode | 行为 |
 * |---|---|
 * | `merge`（默认，ADR-021） | 按主键覆盖，本机独有的记录保留 |
 * | `replace` | **先清空包里确实含有的类别**，再写入 |
 *
 * ⚠️ `replace` **只清包里含有的类别**：包里没带「计算历史」（导出时没勾），
 * 本机历史就保持原样 —— 否则「导入一个只含公式的包」会把历史清空。
 */

import { invoke } from './invoke'
import type {
  BackupInspectResult,
  BackupSelection,
  ImportMode,
  ImportReport,
  LocalExportResult,
} from '@/types/backup'

/**
 * 打包并落到系统「下载」目录（同名自动加序号）。
 *
 * 下载目录不可用时回落应用私有 `backups/`。
 *
 * @param password 备份包**加密密码**；不传 = 不加密（老版本也能读）
 */
export function backupLocalExport(
  selection: BackupSelection,
  password?: string,
): Promise<LocalExportResult> {
  return invoke<LocalExportResult>('backup_local_export', { selection, password })
}

/**
 * 只解析清单不导入（供二次确认）。
 *
 * @param password 备份包加密密码
 *
 * ⚠️ 加密包**没给密码 / 密码不对**时返回的 `decoded` 是 `false`，
 * **不是抛错** —— 前端据此弹「请输入密码」并带上密码重调。
 */
export function backupInspect(
  path: string,
  password?: string,
): Promise<BackupInspectResult> {
  return invoke<BackupInspectResult>('backup_inspect', { path, password })
}

/**
 * 导入本机备份包。
 *
 * @param mode `'merge'`（默认）或 `'replace'`
 */
export function backupLocalImport(
  path: string,
  mode: ImportMode,
  password?: string,
): Promise<ImportReport> {
  return invoke<ImportReport>('backup_local_import', { path, mode, password })
}
