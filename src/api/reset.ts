/**
 * 组 14：重置软件（3 个命令）。
 *
 * 见 docs/08-IPC契约.md §3 组 14。
 *
 * ## ⚠️ 不可撤销
 *
 * `resetExecute` 没有回滚。调用前必须：
 * 1. `resetConfirmLines(selection)` 拿到逐项说明并让用户确认
 * 2. 二次确认弹窗里明确写出「不可撤销、云端备份不受影响」
 *
 * 云端备份（WebDAV 上的包）**一条都不碰** —— 重置只动本机数据。
 */

import { invoke } from './invoke'
import type { ResetCounts, ResetSelection, ResetSummary } from '@/types/reset'

/** 本机真实存量（弹窗每行副标题 + 二次确认里的「将删除 N 条」） */
export function resetCounts(): Promise<ResetCounts> {
  return invoke<ResetCounts>('reset_counts')
}

/**
 * 二次确认文案：逐项写清将删除什么。
 *
 * **只列勾选的类别**；本机没有的也照实写出来（如「公式数据（本机暂无）」）。
 * 文案由后端生成，保证与 `resetExecute` 的实际行为同源。
 */
export function resetConfirmLines(selection: ResetSelection): Promise<string[]> {
  return invoke<string[]>('reset_confirm_lines', { selection })
}

/** 按勾选执行重置，返回实际清掉的量 */
export function resetExecute(selection: ResetSelection): Promise<ResetSummary> {
  return invoke<ResetSummary>('reset_execute', { selection })
}
