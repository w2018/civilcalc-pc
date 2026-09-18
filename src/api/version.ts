/**
 * 组 6：公式版本。
 *
 * 见 docs/08-IPC契约.md §3 组 6 与 ADR-014（head 语义）。
 */

import { invoke } from './invoke'
import type { FormulaSchema, FormulaVersion, VersionDiff } from '@/types/domain'

/** 版本链，顺序 **旧 → 新** */
export function versionList(formulaId: string): Promise<FormulaVersion[]> {
  return invoke<FormulaVersion[]>('version_list', { formulaId })
}

/** 取指定版本；版本号不存在 → `notFound` */
export function versionGet(formulaId: string, version: string): Promise<FormulaVersion> {
  return invoke<FormulaVersion>('version_get', { formulaId, version })
}

/**
 * 存储的 head 版本号；从未切过则 `null`（表示"最新即 head"）。
 *
 * 徽标展示请用 {@link versionResolveHead}，它把 `null` 解析成实际版本。
 */
export function versionHead(formulaId: string): Promise<string | null> {
  return invoke<string | null>('version_head', { formulaId })
}

/**
 * 切换 head 到指定版本。
 *
 * ⚠️ **不改写历史** —— 只更新 `user_formulas.headVersion` 一列。
 */
export function versionSwitch(formulaId: string, version: string): Promise<void> {
  return invoke<void>('version_switch', { formulaId, version })
}

/**
 * 基于给定 schema 新建版本并设为 head。
 *
 * `changeType` 由后端自动推断：空链 → `create`，否则 `edit`。
 */
export function versionCreate(
  schema: FormulaSchema,
  changeLog: string,
  editor: string,
): Promise<FormulaVersion> {
  return invoke<FormulaVersion>('version_create', { schema, changeLog, editor })
}

/** 比较两个版本的 6 维差异（`oldValue` 来自 A，`newValue` 来自 B） */
export function versionDiff(
  formulaId: string,
  versionA: string,
  versionB: string,
): Promise<VersionDiff> {
  return invoke<VersionDiff>('version_diff', { formulaId, versionA, versionB })
}

/**
 * 解析**实际生效**的版本（`head = null` → 最新）。
 *
 * 与 {@link versionHead} 的区别：那个返回存储原值，这个返回解析结果。
 */
export function versionResolveHead(formulaId: string): Promise<FormulaVersion | null> {
  return invoke<FormulaVersion | null>('version_resolve_head', { formulaId })
}
