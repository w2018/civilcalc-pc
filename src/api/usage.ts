/**
 * 组 11：Token 用量统计。
 *
 * 见 docs/08-IPC契约.md §3 组 11。
 */

import { invoke } from './invoke'
import type { UsageGroupBy, UsageStats } from '@/types/system'

/**
 * 用量统计。
 *
 * **一次返回三种视图**（总计 + 按模型 + 按天），页面只发一次请求即可。
 *
 * @param groupBy 只影响返回里的 `groupBy` 回显字段
 */
export function usageStats(groupBy?: UsageGroupBy): Promise<UsageStats> {
  return invoke<UsageStats>('usage_stats', { groupBy })
}

/**
 * 清空用量统计。**必须传 `confirm: true`**，否则后端返回 `invalidArgument`。
 *
 * @returns 清掉的记录条数
 */
export function usageClear(confirm: boolean): Promise<number> {
  return invoke<number>('usage_clear', { confirm })
}

/**
 * 导出用量明细为 CSV。
 *
 * 文件**带 UTF-8 BOM**，中文 Windows 的 Excel 双击直接打开不会乱码。
 *
 * @returns 写入的数据行数（不含表头）
 */
export function usageExportCsv(path: string): Promise<number> {
  return invoke<number>('usage_export_csv', { path })
}
