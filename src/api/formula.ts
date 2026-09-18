/**
 * 组 3：公式 CRUD 与参数草稿。
 *
 * 见 docs/08-IPC契约.md §3 组 3。
 */

import { invoke } from './invoke'
import type { FormulaSchema } from '@/types/domain'
import type { FormulaFilter } from '@/types/system'

/** 按 id 取公式；不存在返回 `null`（**不是错误**） */
export function formulaGet(id: string): Promise<FormulaSchema | null> {
  return invoke<FormulaSchema | null>('formula_get', { id })
}

/** 列表（可按领域 / 来源 / 收藏筛选） */
export function formulaList(filter?: FormulaFilter): Promise<FormulaSchema[]> {
  return invoke<FormulaSchema[]>('formula_list', { filter })
}

/**
 * 保存公式（新建或覆盖）。
 *
 * 后端会**刷新 `updatedAt`** 为当前时间（列表排序依赖它）；
 * 冲突更新时保留原 `favorite` 与 `createdAt`。
 */
export function formulaSave(schema: FormulaSchema): Promise<void> {
  return invoke<void>('formula_save', { schema })
}

/** 删除公式（**幂等**：不存在也算成功） */
export function formulaDelete(id: string): Promise<void> {
  return invoke<void>('formula_delete', { id })
}

export function formulaExists(id: string): Promise<boolean> {
  return invoke<boolean>('formula_exists', { id })
}

/** 保存某公式的参数草稿。`paramsJson` 传**空串 = 删除**该草稿 */
export function formulaDraftSave(formulaId: string, paramsJson: string): Promise<void> {
  return invoke<void>('formula_draft_save', { formulaId, paramsJson })
}

/** 读某公式的参数草稿；无草稿返回 `null` */
export function formulaDraftLoad(formulaId: string): Promise<string | null> {
  return invoke<string | null>('formula_draft_load', { formulaId })
}

/**
 * 清掉除 `currentId` 之外的所有参数草稿。
 *
 * @returns 清掉的草稿数
 */
export function formulaDraftClearOthers(currentId: string): Promise<number> {
  return invoke<number>('formula_draft_clear_others', { currentId })
}
