/**
 * 组 5：检索。
 *
 * 见 docs/08-IPC契约.md §3 组 5。
 *
 * ## 命中规则
 *
 * 中文原文 / 全拼 / **首字母缩写** 均可：
 *
 * | 查询 | 能命中「梁正截面受弯承载力」 |
 * |---|---|
 * | `梁正` | ✅ 中文 |
 * | `liangzheng` | ✅ 全拼（前缀即可） |
 * | `lzjmswczl` | ✅ 首字母缩写 |
 *
 * > 🔴 PC 端修正了源项目的一个 bug：源项目首字母检索（`lzjm`）**实际失效**
 * > （`getFirstLetters` 返回大写而 `match` 用小写比对）。PC 端已恢复可用。
 */

import { invoke } from './invoke'
import type { FormulaSchema, SearchSuggestion } from '@/types/domain'

/** 默认返回条数（与后端一致） */
export const SEARCH_DEFAULT_LIMIT = 20
/** 单次查询上限（后端会夹紧） */
export const SEARCH_MAX_LIMIT = 200

/**
 * 完整检索（含评分排序）。
 *
 * ⚠️ **中文查询会双重计分**（检索文本 100 + 名称 50 + 拼音分支 40 = 190）——
 * 这是源项目行为，前端不要"修正"排序。
 *
 * ## ⚠️ 目前**没有 UI 入口**（v1.0.1 删掉整页搜索后）
 *
 * 顶栏的快捷搜索只用 {@link searchSuggest}（轻量建议）。
 * 本函数保留是因为它是契约的一部分（`docs/08` 组 5），
 * 且「公式库」将来若要做服务端分页检索会直接用它
 * （现在的公式库是本地关键词过滤，不发 IPC）。
 */
export function searchFormulas(
  query: string,
  limit = SEARCH_DEFAULT_LIMIT,
): Promise<FormulaSchema[]> {
  return invoke<FormulaSchema[]>('search_formulas', { query, limit })
}

/**
 * 轻量建议（搜索框实时下拉用）。
 *
 * 只返回 `id` / `resultName` / `domain` / `sourceKind` —— 比完整 Schema
 * 少 23 个字段，IPC 负载小得多。**每敲一个字都会调，务必用这个而不是
 * `searchFormulas`**。
 */
export function searchSuggest(
  query: string,
  limit = SEARCH_DEFAULT_LIMIT,
): Promise<SearchSuggestion[]> {
  return invoke<SearchSuggestion[]>('search_suggest', { query, limit })
}

/**
 * 重建检索索引。
 *
 * ⚠️ 索引是**快照**：后端会在启动、`formulaSave`、`formulaDelete` 后自动重建；
 * **导入备份后需要手动调一次**。
 *
 * @returns 重建后的索引条数
 */
export function searchRebuildIndex(): Promise<number> {
  return invoke<number>('search_rebuild_index')
}
