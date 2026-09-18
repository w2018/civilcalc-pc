/**
 * 组 7b：计算历史。
 *
 * 见 docs/08-IPC契约.md §3 组 7。
 */

import { invoke } from './invoke'
import type { HistoryEntry } from '@/types/domain'

/** 单页上限（后端会夹紧到这个值） */
export const HISTORY_MAX_LIMIT = 500

/**
 * 历史列表（按 `createdAt` 倒序）。
 *
 * @param formulaId 传 `undefined` 取**全局历史**（所有公式混排）
 */
export function historyList(
  formulaId: string | undefined,
  limit = 50,
  offset = 0,
): Promise<HistoryEntry[]> {
  return invoke<HistoryEntry[]>('history_list', { formulaId, limit, offset })
}

/**
 * 按 id 取单条历史（**回填用**）。
 *
 * 回填链路：路由只带 `formulaId` + `historyId`，工作台拿到 id 后调这里
 * 取回 `inputsJson` 预填参数 —— **不把大 JSON 塞进路由**。
 *
 * 历史不存在（已被清理）返回 `null`，**不是错误**。
 */
export function historyGet(id: number): Promise<HistoryEntry | null> {
  return invoke<HistoryEntry | null>('history_get', { id })
}

/**
 * 记一条「公式刚被创建」的历史（**无输入、无结果**）—— 需求 7。
 *
 * ## 什么时候调
 *
 * **只在公式是「新建」的时候**：AI 生成后保存、微调后另存为新公式、
 * 本地解析导入。编辑既有公式**不要**调 —— 那会在历史里刷出一堆
 * 「没算过」的重复条目。
 *
 * ## 为什么由前端决定调不调
 *
 * `formula_save` 同时服务「新建」与「编辑」，后端分不出是哪一种；
 * 「这是一条新公式」只有调用方知道。
 *
 * @param formulaId 必须**已经在库里**（后端从库读快照，保证与当前一致）
 * @param thinking AI 生成 / 微调时的思考全文；本地解析不传
 * @returns 新历史条目的 id
 */
export function historyRecordCreated(formulaId: string, thinking?: string): Promise<number> {
  return invoke<number>('history_record_created', {
    formulaId,
    thinking: thinking?.trim() ? thinking : undefined,
  })
}

/** 删除单条历史（**幂等**） */
export function historyDelete(id: number): Promise<void> {
  return invoke<void>('history_delete', { id })
}

/**
 * 清空全部历史。**必须传 `confirm: true`**，否则后端返回 `invalidArgument`。
 *
 * @returns 清掉的条数
 */
export function historyClear(confirm: boolean): Promise<number> {
  return invoke<number>('history_clear', { confirm })
}

/**
 * 取某公式最近一次历史的思考内容。
 *
 * ⚠️ **不过滤空值**：最新那条没有思考内容就返回 `null`
 * （对齐源项目 `getLatestThinkingContent`）。
 */
export function historyThinking(formulaId: string): Promise<string | null> {
  return invoke<string | null>('history_thinking', { formulaId })
}
